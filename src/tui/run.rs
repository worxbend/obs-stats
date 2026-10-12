//! Terminal lifecycle and the event loop.
//!
//! This is the only module that touches the terminal. It owns raw mode and
//! the alternate screen, and restores both on *every* exit path — including a
//! panic, via a hook installed before raw mode is entered. A TUI that leaves
//! a terminal in raw mode after crashing is a terminal the user has to
//! `reset`, so the guard matters more than the crash.

use std::io::{self, Stdout};
use std::time::{Duration, Instant};

use crossterm::event::{DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use futures_util::StreamExt as _;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::config::Config;
use crate::error::{Error, Result};
use crate::notify::Notifier;
use crate::obs::client::{self, Command, ConnectionParams};
use crate::tui::app::App;
use crate::tui::keys::{self, Pending, Resolution};
use crate::tui::mouse;
use crate::tui::ui;

/// Redraw cadence. Fast enough for the blinking LIVE badge to look alive,
/// slow enough to stay invisible in `top`.
const FRAME_INTERVAL: Duration = Duration::from_millis(100);

type Backend = CrosstermBackend<Stdout>;

/// Run the dashboard until the user quits.
pub async fn run(config: Config, params: ConnectionParams) -> Result<()> {
    let mut app = App::new(config);
    let (mut updates, commands) = client::spawn(params);

    let mut terminal = enter(app.config.ui.mouse)?;
    // Started before the loop so a notification daemon that is slow to answer
    // cannot delay the first frame.
    let notifier = Notifier::spawn(app.config.alerts.desktop && app.config.alerts.enabled);

    let result = event_loop(&mut terminal, &mut app, &mut updates, &commands, &notifier).await;
    leave(&mut terminal, app.config.ui.mouse)?;

    // Best effort: the task may already be gone if it hit a fatal error.
    let _ = commands.send(Command::Shutdown).await;

    result
}

async fn event_loop(
    terminal: &mut Terminal<Backend>,
    app: &mut App,
    updates: &mut tokio::sync::mpsc::Receiver<client::Update>,
    commands: &tokio::sync::mpsc::Sender<Command>,
    notifier: &Notifier,
) -> Result<()> {
    let mut events = EventStream::new();
    event_loop_with_events(terminal, app, updates, commands, notifier, &mut events).await
}

/// The event stream is injectable so scheduling is tested without a TTY.
async fn event_loop_with_events<B, E>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    updates: &mut tokio::sync::mpsc::Receiver<client::Update>,
    commands: &tokio::sync::mpsc::Sender<Command>,
    notifier: &Notifier,
    events: &mut E,
) -> Result<()>
where
    B: ratatui::backend::Backend,
    E: futures_util::Stream<Item = io::Result<Event>> + Unpin,
{
    let mut updates_open = true;
    let mut frames = tokio::time::interval(FRAME_INTERVAL);
    frames.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // Draw once before awaiting anything, so the connecting state is visible
    // immediately rather than after the first tick.
    terminal
        .draw(|frame| ui::draw(frame, app))
        .map_err(|error| Error::Terminal(error.to_string()))?;

    loop {
        // Acquisition can push audio meters much faster than the terminal
        // needs frames. Fold every update into the model, but coalesce their
        // rendering at the frame clock; keys and resize still draw immediately.
        let redraw = tokio::select! {
            biased;

            event = events.next() => {
                match event {
                    Some(Ok(event)) => {
                        if let Some(command) = handle_event(app, event) {
                            // A closed channel means the session task is gone;
                            // the disconnect update will explain why.
                            let _ = commands.send(command).await;
                        }
                    }
                    // The terminal's input stream died: nothing more can be
                    // read, so exiting beats spinning on an error forever.
                    Some(Err(error)) => return Err(Error::Terminal(error.to_string())),
                    None => return Ok(()),
                }
                true
            }

            // Place the ready frame clock ahead of the update channel so a
            // sustained meter-event burst cannot starve rendering.
            _ = frames.tick() => true,

            update = updates.recv(), if updates_open => {
                let update = match update {
                    Some(update) => Some(update),
                    None => {
                        // A closed receiver is permanently ready. Fuse it after
                        // one observation or it burns a core between frames.
                        updates_open = false;
                        match app.snapshot.connection {
                            crate::metrics::ConnectionState::Fatal { .. }
                            | crate::metrics::ConnectionState::Disconnected { .. } => None,
                            _ => Some(client::Update::Disconnected {
                                reason: "the OBS session task stopped".into(),
                                hint: None,
                                retry_in: Duration::ZERO,
                            }),
                        }
                    }
                };

                // Sending is fire-and-forget: the notifier owns its own thread,
                // so a wedged desktop cannot stall a frame.
                if let Some(update) = update {
                    for alert in app.apply(update, Instant::now()) {
                        notifier.send(&alert);
                    }
                }
                false
            }
        };

        if app.should_quit {
            return Ok(());
        }
        if !redraw {
            continue;
        }

        app.tick(Instant::now());
        terminal
            .draw(|frame| ui::draw(frame, app))
            .map_err(|error| Error::Terminal(error.to_string()))?;
    }
}

/// Translate a terminal event into an app action.
///
/// Returns the OBS command the action produced, if any.
fn handle_event(app: &mut App, event: Event) -> Option<Command> {
    let now = Instant::now();
    match event {
        Event::Key(key) => {
            // Windows reports both press and release; acting on both would
            // double every keystroke.
            if key.kind != KeyEventKind::Press {
                return None;
            }
            match keys::resolve(app.overlay, app.pending, key) {
                Resolution::Act(action) => {
                    // The sequence completed, so nothing is outstanding — and
                    // it must be cleared *before* the action runs, or an
                    // action that opens an overlay inherits a stale prefix.
                    app.pending = Pending::None;
                    app.handle(action, now)
                }
                Resolution::Wait(pending) => {
                    app.pending = pending;
                    None
                }
                Resolution::Ignore => {
                    app.pending = Pending::None;
                    None
                }
            }
        }
        Event::Mouse(mouse) if app.config.ui.mouse => {
            mouse::mouse_event(app, mouse.kind, mouse.column, mouse.row, now)
        }
        // Resize redraws on the next loop iteration; nothing to compute here.
        _ => None,
    }
}

/// Enter raw mode and the alternate screen, installing a restoring panic hook.
fn enter(mouse: bool) -> Result<Terminal<Backend>> {
    install_panic_hook();

    enable_raw_mode().map_err(|error| Error::Terminal(error.to_string()))?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).map_err(|error| Error::Terminal(error.to_string()))?;
    if mouse {
        execute!(stdout, EnableMouseCapture).map_err(|error| Error::Terminal(error.to_string()))?;
    }

    Terminal::new(CrosstermBackend::new(stdout)).map_err(|error| Error::Terminal(error.to_string()))
}

/// Restore the terminal. Every step is best-effort: a failure here must not
/// mask the error that caused the exit.
fn leave(terminal: &mut Terminal<Backend>, mouse: bool) -> Result<()> {
    let _ = disable_raw_mode();
    if mouse {
        let _ = execute!(terminal.backend_mut(), DisableMouseCapture);
    }
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let _ = terminal.show_cursor();
    Ok(())
}

/// Restore the terminal before a panic prints, so the backtrace is readable
/// and the shell is usable afterwards.
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen);
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use crossterm::event::{MouseButton, MouseEventKind};

    use crate::tui::view::View;

    use super::*;

    fn app() -> App {
        App::new(Config::default())
    }

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    /// Draw one frame, so the app's click regions describe something real.
    ///
    /// Every mouse test goes through this: hit-testing against hand-written
    /// rects would pass while the actual layout put the widget elsewhere.
    fn drawn(app: &mut App, width: u16, height: u16) {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| ui::draw(frame, app)).unwrap();
    }

    fn click_at(app: &mut App, column: u16, row: u16) -> Option<Command> {
        mouse::mouse_event(
            app,
            MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            Instant::now(),
        )
    }

    /// A log view with `count` entries, drawn and ready to click.
    fn with_log(count: usize) -> App {
        use crate::metrics::LogEntry;
        use crate::obs::client::LogLevel;

        let mut app = app();
        app.view = View::Log;
        app.snapshot.log = (0..count)
            .map(|index| LogEntry {
                level: LogLevel::Info,
                message: format!("event {index}"),
                at: Duration::from_secs(index as u64),
            })
            .collect();
        drawn(&mut app, 120, 40);
        app
    }

    #[tokio::test]
    async fn telemetry_bursts_are_folded_without_rendering_each_update() {
        let mut app = app();
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let (tx, mut updates) = tokio::sync::mpsc::channel(256);
        for _ in 0..192 {
            tx.try_send(client::Update::Poll(Box::default())).unwrap();
        }
        let (commands, _receiver) = tokio::sync::mpsc::channel(4);
        let notifier = Notifier::spawn(false);
        let mut events = futures_util::stream::pending();
        let result = tokio::time::timeout(
            Duration::from_millis(220),
            event_loop_with_events(
                &mut terminal,
                &mut app,
                &mut updates,
                &commands,
                &notifier,
                &mut events,
            ),
        )
        .await;
        assert!(
            result.is_err(),
            "the pending input stream should still be running"
        );
        assert_eq!(app.snapshot.polls, 192, "all acquisition updates must land");
        assert!(
            app.frame <= 4,
            "updates caused {} frames instead of clock-coalesced rendering",
            app.frame
        );
    }

    #[tokio::test]
    async fn a_closed_update_channel_preserves_the_original_fatal_reason() {
        let mut app = app();
        app.apply(
            client::Update::Fatal {
                reason: "authentication failed".into(),
                hint: Some("check credentials"),
            },
            Instant::now(),
        );
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let (tx, mut updates) = tokio::sync::mpsc::channel(4);
        drop(tx);
        let (commands, _receiver) = tokio::sync::mpsc::channel(4);
        let notifier = Notifier::spawn(false);
        let mut events = futures_util::stream::pending();
        assert!(
            tokio::time::timeout(
                Duration::from_millis(120),
                event_loop_with_events(
                    &mut terminal,
                    &mut app,
                    &mut updates,
                    &commands,
                    &notifier,
                    &mut events
                )
            )
            .await
            .is_err()
        );
        assert!(matches!(&app.snapshot.connection,
            crate::metrics::ConnectionState::Fatal { reason, .. } if reason == "authentication failed"));
        assert_eq!(
            app.snapshot.log.len(),
            1,
            "closed channel repeated terminal updates"
        );
    }

    #[test]
    fn key_releases_are_ignored_so_keys_do_not_double_fire() {
        use crossterm::event::{KeyCode, KeyEvent, KeyEventState, KeyModifiers};

        let mut app = app();
        let release = Event::Key(KeyEvent::new_with_kind_and_state(
            KeyCode::Tab,
            KeyModifiers::NONE,
            KeyEventKind::Release,
            KeyEventState::NONE,
        ));
        handle_event(&mut app, release);
        assert_eq!(app.view, View::Overview, "a release must not switch views");

        let press = Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        handle_event(&mut app, press);
        assert_eq!(app.view, View::Performance);
    }

    #[test]
    fn quitting_via_a_key_produces_a_shutdown_command() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut app = app();
        let command = handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)),
        );
        assert!(app.should_quit);
        assert!(matches!(command, Some(Command::Shutdown)));
    }

    #[test]
    fn mouse_events_are_ignored_when_the_mouse_is_disabled() {
        use crossterm::event::{MouseEvent, MouseEventKind};

        let mut config = Config::default();
        config.ui.mouse = false;
        let mut app = App::new(config);
        app.view = View::Log;
        app.set_row_count(View::Log, 20);

        handle_event(
            &mut app,
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: 0,
                row: 0,
                modifiers: crossterm::event::KeyModifiers::NONE,
            }),
        );
        assert_eq!(app.selection(View::Log), 0);
    }

    #[test]
    fn resize_events_are_harmless() {
        let mut app = app();
        assert!(handle_event(&mut app, Event::Resize(10, 10)).is_none());
        assert!(!app.should_quit);
    }

    #[test]
    fn a_click_abandons_a_half_typed_key_sequence() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut app = with_log(30);
        handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)),
        );
        assert_eq!(app.pending, Pending::Leader);

        click_at(&mut app, 5, 5);
        assert_eq!(app.pending, Pending::None);
    }
}
