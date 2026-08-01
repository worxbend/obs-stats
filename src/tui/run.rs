//! Terminal lifecycle and the event loop.
//!
//! This is the only module that touches the terminal. It owns raw mode and
//! the alternate screen, and restores both on *every* exit path — including a
//! panic, via a hook installed before raw mode is entered. A TUI that leaves
//! a terminal in raw mode after crashing is a terminal the user has to
//! `reset`, so the guard matters more than the crash.

use std::io::{self, Stdout};
use std::time::{Duration, Instant};

use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyEventKind, MouseButton,
    MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use futures_util::StreamExt as _;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::config::Config;
use crate::error::{Error, Result};
use crate::obs::client::{self, Command, ConnectionParams};
use crate::tui::app::App;
use crate::tui::keys::{self, Action};
use crate::tui::ui;
use crate::tui::view::View;

/// Redraw cadence. Fast enough for the blinking LIVE badge to look alive,
/// slow enough to stay invisible in `top`.
const FRAME_INTERVAL: Duration = Duration::from_millis(100);

type Backend = CrosstermBackend<Stdout>;

/// Run the dashboard until the user quits.
pub async fn run(config: Config, params: ConnectionParams) -> Result<()> {
    let mut app = App::new(config);
    let (mut updates, commands) = client::spawn(params);

    let mut terminal = enter(app.config.ui.mouse)?;
    let result = event_loop(&mut terminal, &mut app, &mut updates, &commands).await;
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
) -> Result<()> {
    let mut events = EventStream::new();
    let mut frames = tokio::time::interval(FRAME_INTERVAL);
    frames.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // Draw once before awaiting anything, so the connecting state is visible
    // immediately rather than after the first tick.
    terminal
        .draw(|frame| ui::draw(frame, app))
        .map_err(|error| Error::Terminal(error.to_string()))?;

    loop {
        tokio::select! {
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
            }

            update = updates.recv() => {
                match update {
                    Some(update) => app.apply(update, Instant::now()),
                    // The session task ended without telling us; treat that as
                    // a disconnect rather than quitting the UI out from under
                    // the user.
                    None => app.apply(
                        client::Update::Disconnected {
                            reason: "the OBS session task stopped".into(),
                            hint: None,
                            retry_in: Duration::from_secs(0),
                        },
                        Instant::now(),
                    ),
                }
            }

            _ = frames.tick() => {}
        }

        if app.should_quit {
            return Ok(());
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
            let action = keys::resolve(app.overlay, key)?;
            app.handle(action, now)
        }
        Event::Mouse(mouse) if app.config.ui.mouse => {
            let action = mouse_action(app, mouse.kind, mouse.column, mouse.row)?;
            app.handle(action, now)
        }
        // Resize redraws on the next loop iteration; nothing to compute here.
        _ => None,
    }
}

/// Map a mouse event to an action.
///
/// Scrolling moves the list cursor; a click on the tab strip selects that
/// view. Anything else is ignored so stray clicks cannot change state.
fn mouse_action(app: &App, kind: MouseEventKind, column: u16, row: u16) -> Option<Action> {
    match kind {
        MouseEventKind::ScrollDown => Some(Action::Down),
        MouseEventKind::ScrollUp => Some(Action::Up),
        MouseEventKind::Down(MouseButton::Left) => {
            // The tab strip sits directly under the two-row header.
            if row != crate::tui::widgets::header::HEIGHT {
                return None;
            }
            tab_at(app, column).map(Action::SelectView)
        }
        _ => None,
    }
}

/// Which tab occupies terminal column `column`.
///
/// Mirrors the layout `Tabs` uses: one leading space, then each title, with a
/// ` │ ` divider between them.
pub fn tab_at(app: &App, column: u16) -> Option<View> {
    let mut cursor: u16 = 1;
    for view in View::ALL {
        let label_width = tab_label_width(view, app.show_icons);
        if column >= cursor && column < cursor + label_width {
            return Some(view);
        }
        cursor += label_width + 3; // ` │ `
    }
    None
}

/// Display width of one tab's label, matching what [`ui`] renders.
fn tab_label_width(view: View, show_icons: bool) -> u16 {
    // "N " prefix, then the title, plus "icon " when icons are on. The icons
    // are wide glyphs that occupy two columns.
    let icon = if show_icons { 3 } else { 0 };
    2 + icon + view.title().chars().count() as u16
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
    use super::*;

    fn app() -> App {
        App::new(Config::default())
    }

    #[test]
    fn scrolling_moves_the_list_cursor() {
        let app = app();
        assert_eq!(
            mouse_action(&app, MouseEventKind::ScrollDown, 0, 0),
            Some(Action::Down)
        );
        assert_eq!(
            mouse_action(&app, MouseEventKind::ScrollUp, 0, 0),
            Some(Action::Up)
        );
    }

    #[test]
    fn clicking_the_tab_strip_selects_that_view() {
        let app = app();
        let row = crate::tui::widgets::header::HEIGHT;
        assert_eq!(
            mouse_action(&app, MouseEventKind::Down(MouseButton::Left), 2, row),
            Some(Action::SelectView(View::Overview))
        );
    }

    #[test]
    fn clicking_outside_the_tab_strip_does_nothing() {
        let app = app();
        let elsewhere = crate::tui::widgets::header::HEIGHT + 5;
        assert_eq!(
            mouse_action(&app, MouseEventKind::Down(MouseButton::Left), 2, elsewhere),
            None
        );
    }

    #[test]
    fn right_clicks_and_drags_are_ignored() {
        let app = app();
        let row = crate::tui::widgets::header::HEIGHT;
        assert_eq!(
            mouse_action(&app, MouseEventKind::Down(MouseButton::Right), 2, row),
            None
        );
        assert_eq!(
            mouse_action(&app, MouseEventKind::Drag(MouseButton::Left), 2, row),
            None
        );
    }

    #[test]
    fn tab_hit_testing_walks_the_strip_in_order() {
        let app = app();
        let mut seen = Vec::new();
        for column in 0..200 {
            if let Some(view) = tab_at(&app, column)
                && seen.last() != Some(&view)
            {
                seen.push(view);
            }
        }
        assert_eq!(seen, View::ALL.to_vec(), "tabs must be hit in strip order");
    }

    #[test]
    fn columns_past_the_last_tab_hit_nothing() {
        let app = app();
        assert_eq!(tab_at(&app, 250), None);
        assert_eq!(tab_at(&app, 0), None, "the leading pad is not a tab");
    }

    #[test]
    fn hiding_icons_narrows_every_tab() {
        let with = tab_label_width(View::Overview, true);
        let without = tab_label_width(View::Overview, false);
        assert!(without < with);
        assert_eq!(without, 2 + View::Overview.title().len() as u16);
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
}
