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
use crate::notify::Notifier;
use crate::obs::client::{self, Command, ConnectionParams};
use crate::tui::app::{self, App, Hit};
use crate::tui::keys::{self, Action, Overlay, Pending, Resolution};
use crate::tui::ui;
use crate::tui::view::View;
use crate::tui::widgets::scrollbar;

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
                let update = update.unwrap_or_else(||
                    // The session task ended without telling us; treat that as
                    // a disconnect rather than quitting the UI out from under
                    // the user.
                    client::Update::Disconnected {
                        reason: "the OBS session task stopped".into(),
                        hint: None,
                        retry_in: Duration::from_secs(0),
                    }
                );

                // Sending is fire-and-forget: the notifier owns its own thread,
                // so a wedged desktop cannot stall a frame.
                for alert in app.apply(update, Instant::now()) {
                    notifier.send(&alert);
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
            mouse_event(app, mouse.kind, mouse.column, mouse.row, now)
        }
        // Resize redraws on the next loop iteration; nothing to compute here.
        _ => None,
    }
}

/// Apply a mouse event.
///
/// Some clicks move a cursor rather than producing an action — a list row, a
/// theme row — so this owns the app rather than returning an [`Action`] the
/// way the keymap does.
fn mouse_event(
    app: &mut App,
    kind: MouseEventKind,
    column: u16,
    row: u16,
    now: Instant,
) -> Option<Command> {
    // A stray click anywhere abandons a half-typed key sequence, the same way
    // an unmapped key does.
    if !matches!(kind, MouseEventKind::Moved) {
        app.pending = Pending::None;
    }

    match kind {
        MouseEventKind::ScrollDown => app.handle(Action::Down, now),
        MouseEventKind::ScrollUp => app.handle(Action::Up, now),
        MouseEventKind::Down(MouseButton::Left) => click(app, column, row, now),
        MouseEventKind::Drag(MouseButton::Left) => {
            drag(app, column, row);
            None
        }
        _ => None,
    }
}

/// Resolve a left click against whatever was drawn under it.
///
/// Overlays are tested first: while one is open it owns the screen, so a click
/// that reaches the dashboard beneath would act on something the user cannot
/// see.
fn click(app: &mut App, column: u16, row: u16, now: Instant) -> Option<Command> {
    if app.overlay != Overlay::None {
        return click_overlay(app, column, row, now);
    }

    // The tab strip.
    if app.regions.tabs.height > 0
        && row == app.regions.tabs.y
        && let Some(view) = tab_at(app, column)
    {
        return app.handle(Action::SelectView(view), now);
    }

    // The scrollbar, before the list: it sits on the panel's border, one
    // column outside the rows themselves.
    if let Some(track) = app.regions.scrollbar
        && app::contains(track, column, row)
    {
        scrub(app, row);
        return None;
    }

    if let Some(hit) = app.regions.hit(column, row) {
        return match hit {
            Hit::Pause => app.handle(Action::TogglePause, now),
            // Clicking the cadence walks it faster, which is the direction
            // someone poking at it almost always wants; it wraps at the floor.
            Hit::Poll => app.handle(Action::FasterPolling, now),
            Hit::ThemeName => app.handle(Action::ToggleThemePicker, now),
            Hit::Alerts => app.handle(Action::SelectView(View::Log), now),
        };
    }

    app.click_row(row);
    None
}

/// Resolve a click while an overlay is open.
fn click_overlay(app: &mut App, column: u16, row: u16, now: Instant) -> Option<Command> {
    // Outside the panel dismisses it, which is what clicking away from a
    // modal means everywhere else.
    if let Some(panel) = app.regions.overlay_panel
        && !app::contains(panel, column, row)
    {
        return app.handle(Action::Cancel, now);
    }

    // On a row: the first click selects, and a click on the already-selected
    // row commits — the same two-step as moving the cursor and pressing Enter.
    if app.regions.overlay_list.is_some() {
        let before = overlay_cursor(app);
        if app.click_overlay_row(row) {
            if overlay_cursor(app) == before {
                return app.handle(Action::Confirm, now);
            }
            return None;
        }
    }
    None
}

/// The cursor of whichever overlay list is open.
fn overlay_cursor(app: &App) -> usize {
    match app.overlay {
        Overlay::Themes => app.theme_cursor,
        Overlay::Palette => app.palette.cursor,
        _ => 0,
    }
}

/// Move the selection to wherever the scrollbar was grabbed.
fn scrub(app: &mut App, row: u16) {
    let Some(track) = app.regions.scrollbar else {
        return;
    };
    let total = app.row_count(app.view);
    if total == 0 {
        return;
    }
    let target = scrollbar::row_at(track.height, total, row.saturating_sub(track.y));
    app.select_row(target);
}

/// A left-button drag: scrubbing the scrollbar, or sweeping the selection
/// through a list.
fn drag(app: &mut App, column: u16, row: u16) {
    if app.overlay != Overlay::None {
        app.click_overlay_row(row);
        return;
    }
    if let Some(track) = app.regions.scrollbar
        && column >= track.x
        && column < track.x + track.width
    {
        scrub(app, row);
        return;
    }
    app.click_row(row);
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
        mouse_event(
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

    #[test]
    fn scrolling_moves_the_list_cursor() {
        let mut app = with_log(30);
        mouse_event(&mut app, MouseEventKind::ScrollDown, 0, 0, Instant::now());
        assert_eq!(app.selection(View::Log), 1);
        mouse_event(&mut app, MouseEventKind::ScrollUp, 0, 0, Instant::now());
        assert_eq!(app.selection(View::Log), 0);
    }

    #[test]
    fn clicking_the_tab_strip_selects_that_view() {
        let mut app = app();
        drawn(&mut app, 120, 40);
        let row = app.regions.tabs.y;

        click_at(&mut app, 2, row);
        assert_eq!(app.view, View::Overview);

        // Far enough along the strip to land on a later tab.
        let mut seen = None;
        for column in 0..100 {
            if let Some(view) = tab_at(&app, column)
                && view == View::Scenes
            {
                seen = Some(column);
                break;
            }
        }
        click_at(&mut app, seen.expect("Scenes must be on the strip"), row);
        assert_eq!(app.view, View::Scenes);
    }

    #[test]
    fn clicking_a_log_row_moves_the_cursor_to_it() {
        let mut app = with_log(30);
        let list = app.regions.list.expect("the log draws a list");

        click_at(&mut app, list.x + 2, list.y + 3);
        assert_eq!(app.selection(View::Log), 3);
    }

    #[test]
    fn clicking_below_the_last_row_leaves_the_cursor_alone() {
        let mut app = with_log(3);
        let list = app.regions.list.unwrap();
        click_at(&mut app, list.x + 2, list.y + 1);
        assert_eq!(app.selection(View::Log), 1);

        // Well past the third and final entry, but still inside the panel.
        click_at(&mut app, list.x + 2, list.y + 10);
        assert_eq!(app.selection(View::Log), 1, "an empty row is not a target");
    }

    #[test]
    fn a_click_on_a_view_without_a_list_changes_nothing() {
        let mut app = app();
        app.view = View::Overview;
        drawn(&mut app, 120, 40);
        assert!(app.regions.list.is_none());
        assert!(click_at(&mut app, 40, 20).is_none());
        assert_eq!(app.selection(View::Overview), 0);
    }

    #[test]
    fn dragging_sweeps_the_selection_through_a_list() {
        let mut app = with_log(30);
        let list = app.regions.list.unwrap();

        click_at(&mut app, list.x + 2, list.y);
        for offset in 1..5 {
            mouse_event(
                &mut app,
                MouseEventKind::Drag(MouseButton::Left),
                list.x + 2,
                list.y + offset,
                Instant::now(),
            );
        }
        assert_eq!(app.selection(View::Log), 4);
    }

    #[test]
    fn a_long_list_gets_a_scrollbar_that_can_be_scrubbed() {
        let mut app = with_log(200);
        let track = app
            .regions
            .scrollbar
            .expect("200 rows in a 40-row terminal must scroll");

        // Grab the very bottom of the track.
        click_at(&mut app, track.x, track.y + track.height - 1);
        assert_eq!(app.selection(View::Log), 199, "the end of the list");

        click_at(&mut app, track.x, track.y);
        assert_eq!(app.selection(View::Log), 0);
    }

    #[test]
    fn a_short_list_has_no_scrollbar_to_click() {
        let app = with_log(3);
        assert!(
            app.regions.scrollbar.is_none(),
            "a bar that cannot move is decoration"
        );
    }

    #[test]
    fn clicking_the_pause_indicator_toggles_it_both_ways() {
        let mut app = app();
        drawn(&mut app, 120, 40);
        let (rect, _) = app
            .regions
            .hits
            .iter()
            .find(|(_, hit)| *hit == Hit::Pause)
            .copied()
            .expect("the status bar offers a pause target");

        click_at(&mut app, rect.x, rect.y);
        assert!(app.paused);

        // Redraw: paused renders a wider chip, so the rect moves.
        drawn(&mut app, 120, 40);
        let (rect, _) = app
            .regions
            .hits
            .iter()
            .find(|(_, hit)| *hit == Hit::Pause)
            .copied()
            .unwrap();
        click_at(&mut app, rect.x, rect.y);
        assert!(!app.paused);
    }

    #[test]
    fn clicking_the_poll_rate_speeds_it_up() {
        let mut app = app();
        drawn(&mut app, 120, 40);
        let (rect, _) = app
            .regions
            .hits
            .iter()
            .find(|(_, hit)| *hit == Hit::Poll)
            .copied()
            .unwrap();

        let before = app.poll_interval;
        let command = click_at(&mut app, rect.x, rect.y);
        assert!(app.poll_interval < before);
        assert!(matches!(command, Some(Command::SetPollInterval(_))));
    }

    #[test]
    fn clicking_the_theme_name_opens_the_picker() {
        let mut app = app();
        drawn(&mut app, 120, 40);
        let (rect, _) = app
            .regions
            .hits
            .iter()
            .find(|(_, hit)| *hit == Hit::ThemeName)
            .copied()
            .unwrap();

        click_at(&mut app, rect.x, rect.y);
        assert_eq!(app.overlay, Overlay::Themes);
    }

    #[test]
    fn clicking_the_alert_counter_jumps_to_the_log() {
        let mut app = app();
        app.alert_count = 4;
        drawn(&mut app, 120, 40);
        let (rect, _) = app
            .regions
            .hits
            .iter()
            .find(|(_, hit)| *hit == Hit::Alerts)
            .copied()
            .expect("a raised alert must be clickable");

        click_at(&mut app, rect.x, rect.y);
        assert_eq!(app.view, View::Log);
    }

    #[test]
    fn there_is_no_alert_target_when_nothing_has_been_raised() {
        let mut app = app();
        drawn(&mut app, 120, 40);
        assert!(!app.regions.hits.iter().any(|(_, hit)| *hit == Hit::Alerts));
    }

    #[test]
    fn clicking_a_theme_row_previews_it_and_clicking_again_applies_it() {
        let mut app = app();
        app.handle(Action::ToggleThemePicker, Instant::now());
        drawn(&mut app, 120, 40);
        let list = app.regions.overlay_list.expect("the picker draws a list");

        let before = app.theme.id;
        click_at(&mut app, list.x + 2, list.y + 3);
        assert_ne!(app.theme.id, before, "the first click previews");
        assert_eq!(app.overlay, Overlay::Themes, "and leaves it open");
        let previewed = app.theme.id;

        click_at(&mut app, list.x + 2, list.y + 3);
        assert_eq!(app.overlay, Overlay::None, "the second click applies");
        assert_eq!(app.theme.id, previewed);
        assert_eq!(app.config.ui.theme, previewed);
    }

    #[test]
    fn clicking_outside_an_overlay_dismisses_it() {
        let mut app = app();
        app.handle(Action::ToggleThemePicker, Instant::now());
        drawn(&mut app, 120, 40);
        let panel = app.regions.overlay_panel.unwrap();
        let original = app.theme.id;

        // Move the preview first, so the revert is observable.
        app.handle(Action::Down, Instant::now());
        assert_ne!(app.theme.id, original);

        click_at(&mut app, panel.x.saturating_sub(2), panel.y);
        assert_eq!(app.overlay, Overlay::None);
        assert_eq!(app.theme.id, original, "dismissing reverts the preview");
    }

    #[test]
    fn a_click_inside_an_overlay_never_reaches_the_dashboard_beneath() {
        let mut app = app();
        app.view = View::Log;
        app.handle(Action::ToggleHelp, Instant::now());
        drawn(&mut app, 120, 40);
        let panel = app.regions.overlay_panel.expect("help draws a panel");

        // Dead centre of the help panel, which is over the tab strip's row in
        // neither position — but the point stands: the view must not change.
        click_at(&mut app, panel.x + panel.width / 2, panel.y + 2);
        assert_eq!(app.view, View::Log);
    }

    #[test]
    fn clicking_a_palette_row_runs_it() {
        let mut app = app();
        app.handle(Action::OpenPaletteFiltered("view"), Instant::now());
        drawn(&mut app, 120, 40);
        let list = app.regions.overlay_list.unwrap();

        click_at(&mut app, list.x + 2, list.y + 2);
        assert_eq!(app.palette.cursor, 2);
        click_at(&mut app, list.x + 2, list.y + 2);
        assert_eq!(app.overlay, Overlay::None);
        assert_eq!(app.view, View::ALL[2]);
    }

    #[test]
    fn right_clicks_are_ignored() {
        let mut app = with_log(30);
        let list = app.regions.list.unwrap();
        assert!(
            mouse_event(
                &mut app,
                MouseEventKind::Down(MouseButton::Right),
                list.x + 2,
                list.y + 3,
                Instant::now(),
            )
            .is_none()
        );
        assert_eq!(app.selection(View::Log), 0, "nothing moved");
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
