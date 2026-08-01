//! Application state and the action handler.
//!
//! `App` holds everything the renderer needs and nothing else — no terminal,
//! no socket, no channels. [`App::handle`] is a pure state transition that
//! returns the OBS command (if any) the shell should forward, which is what
//! lets the entire interaction model be tested without a TTY.

use std::time::{Duration, Instant};

use crate::config::Config;
use crate::config::model::{MAX_POLL_INTERVAL_MS, MIN_POLL_INTERVAL_MS};
use crate::metrics::alerts::{Alert, Severity};
use crate::metrics::{Snapshot, history_capacity};
use crate::obs::client::{Command, Update};
use crate::tui::keys::{Action, Overlay};
use crate::tui::theme::{self, Theme};
use crate::tui::view::View;

/// A transient message shown in the status bar.
#[derive(Debug, Clone, PartialEq)]
pub struct Toast {
    pub message: String,
    pub until: Instant,
}

/// How long a toast stays on screen.
pub const TOAST_DURATION: Duration = Duration::from_secs(3);

/// How long a frame-loss banner stays up.
///
/// Longer than a toast because it reports something that needs acting on, and
/// longest for the ones a viewer can see — but still self-clearing, so a
/// dashboard left running does not accumulate stale warnings.
pub const BANNER_DURATION: Duration = Duration::from_secs(20);
pub const BANNER_DURATION_CRITICAL: Duration = Duration::from_secs(45);

/// An alert currently shown as a banner.
#[derive(Debug, Clone, PartialEq)]
pub struct Banner {
    pub alert: Alert,
    pub until: Instant,
}

pub struct App {
    pub config: Config,
    pub snapshot: Snapshot,
    pub theme: &'static Theme,
    pub view: View,
    pub overlay: Overlay,
    /// Freeze the display without dropping the connection.
    pub paused: bool,
    /// Draw Unicode block glyphs and rounded borders.
    pub unicode: bool,
    pub show_icons: bool,
    /// Current hot-poll cadence.
    pub poll_interval: Duration,
    /// Set when the shell should tear down the terminal and exit.
    pub should_quit: bool,
    /// Monotonic frame counter, used by the header's activity indicator.
    pub frame: u64,
    /// Version shown in the header.
    ///
    /// A field rather than a direct read of [`crate::VERSION`] so the
    /// screenshot generator can pin it: the docs images would otherwise drift
    /// on every release bump, and CI's no-drift check would fail every time.
    pub version: String,
    /// Cursor position per view, indexed by [`View::index`].
    selection: [usize; View::ALL.len()],
    /// How many rows the focused list last rendered, so paging knows its step
    /// and `End` knows where the bottom is.
    row_counts: [usize; View::ALL.len()],
    /// Cursor in the theme picker.
    pub theme_cursor: usize,
    /// Theme to restore if the picker is cancelled.
    theme_before_preview: &'static Theme,
    pub toast: Option<Toast>,
    /// The frame-loss banner, if one is showing.
    pub banner: Option<Banner>,
    /// Alerts raised this session, newest last. Drives the header counter, so
    /// a loss that happened while you were on another view is still evident.
    pub alert_count: u64,
}

impl App {
    pub fn new(config: Config) -> Self {
        let theme = theme::resolve(&config.ui.theme);
        let view = View::from_id(&config.ui.start_view).unwrap_or(View::Overview);
        let capacity = history_capacity(config.ui.history_seconds, config.ui.poll_interval_ms);

        let mut snapshot = Snapshot::new(capacity);
        snapshot.set_alert_config(config.alerts.to_alert_config());

        Self {
            snapshot,
            theme,
            view,
            overlay: Overlay::None,
            paused: false,
            unicode: config.ui.unicode,
            show_icons: config.ui.show_icons,
            poll_interval: Duration::from_millis(config.ui.poll_interval_ms),
            should_quit: false,
            frame: 0,
            version: crate::VERSION.to_string(),
            selection: [0; View::ALL.len()],
            row_counts: [0; View::ALL.len()],
            theme_cursor: theme::index_of(&config.ui.theme),
            theme_before_preview: theme,
            toast: None,
            banner: None,
            alert_count: 0,
            config,
        }
    }

    /// Fold an update from the OBS task into the snapshot.
    ///
    /// Ignored while paused, except for connection-state changes: a paused
    /// dashboard that silently hides a dropped connection would be a lie.
    ///
    /// Returns any alerts raised, for the shell to hand to the notifier.
    pub fn apply(&mut self, update: Update, now: Instant) -> Vec<Alert> {
        if self.paused
            && matches!(
                update,
                Update::Poll(_) | Update::Inventory(_) | Update::Meters(_)
            )
        {
            return Vec::new();
        }
        let alerts = self.snapshot.apply(update, now);
        self.clamp_selection();

        for alert in &alerts {
            self.raise(alert.clone(), now);
        }
        alerts
    }

    /// Show `alert` as a banner, unless something worse is already up.
    ///
    /// A render-lag warning must not displace a "viewers are seeing stutter"
    /// banner that is still on screen.
    fn raise(&mut self, alert: Alert, now: Instant) {
        self.alert_count += 1;

        if let Some(current) = &self.banner
            && now < current.until
            && current.alert.severity() > alert.severity()
        {
            return;
        }

        let duration = if alert.severity() == Severity::Critical {
            BANNER_DURATION_CRITICAL
        } else {
            BANNER_DURATION
        };
        self.banner = Some(Banner {
            until: now + duration,
            alert,
        });
    }

    /// Apply an action, returning a command for the OBS task if one is due.
    pub fn handle(&mut self, action: Action, now: Instant) -> Option<Command> {
        match action {
            Action::Quit => {
                self.should_quit = true;
                Some(Command::Shutdown)
            }
            Action::ToggleHelp => {
                self.overlay = if self.overlay == Overlay::Help {
                    Overlay::None
                } else {
                    Overlay::Help
                };
                None
            }
            Action::ToggleThemePicker => {
                if self.overlay == Overlay::Themes {
                    self.cancel_overlay();
                } else {
                    self.open_theme_picker();
                }
                None
            }
            Action::NextView => {
                self.view = self.view.next();
                None
            }
            Action::PreviousView => {
                self.view = self.view.previous();
                None
            }
            Action::SelectView(view) => {
                self.view = view;
                None
            }
            Action::Up => {
                self.move_cursor(-1);
                None
            }
            Action::Down => {
                self.move_cursor(1);
                None
            }
            Action::PageUp => {
                self.move_cursor(-(self.page_step() as isize));
                None
            }
            Action::PageDown => {
                self.move_cursor(self.page_step() as isize);
                None
            }
            Action::Home => {
                self.set_cursor(0);
                None
            }
            Action::End => {
                self.set_cursor(usize::MAX);
                None
            }
            Action::TogglePause => {
                self.paused = !self.paused;
                self.notify(
                    if self.paused { "paused" } else { "resumed" }.to_string(),
                    now,
                );
                None
            }
            Action::Refresh => {
                // Refreshing while paused would be a no-op the user cannot
                // see, so resume first — that is what they meant.
                self.paused = false;
                self.notify("refreshing".to_string(), now);
                Some(Command::RefreshNow)
            }
            Action::FasterPolling => self.scale_poll_interval(0.5, now),
            Action::SlowerPolling => self.scale_poll_interval(2.0, now),
            Action::ToggleUnicode => {
                self.unicode = !self.unicode;
                None
            }
            Action::Confirm => {
                self.confirm_overlay(now);
                None
            }
            Action::Cancel => {
                self.cancel_overlay();
                None
            }
        }
    }

    /// Tell the app how many rows the focused list drew, so paging and `End`
    /// have something real to clamp against.
    pub fn set_row_count(&mut self, view: View, rows: usize) {
        self.row_counts[view.index()] = rows;
        self.clamp_selection();
    }

    /// Cursor position for `view`, already clamped to its row count.
    pub fn selection(&self, view: View) -> usize {
        let rows = self.row_counts[view.index()];
        if rows == 0 {
            0
        } else {
            self.selection[view.index()].min(rows - 1)
        }
    }

    /// Expire a toast whose time is up. Called once per frame.
    pub fn tick(&mut self, now: Instant) {
        self.frame = self.frame.wrapping_add(1);
        if let Some(toast) = &self.toast
            && now >= toast.until
        {
            self.toast = None;
        }
        if let Some(banner) = &self.banner
            && now >= banner.until
        {
            self.banner = None;
        }
    }

    /// Show a transient status-bar message.
    pub fn notify(&mut self, message: String, now: Instant) {
        self.toast = Some(Toast {
            message,
            until: now + TOAST_DURATION,
        });
    }

    /// The theme the picker is currently previewing.
    pub fn previewed_theme(&self) -> &'static Theme {
        theme::THEMES
            .get(self.theme_cursor)
            .unwrap_or(theme::FALLBACK)
    }

    fn open_theme_picker(&mut self) {
        self.overlay = Overlay::Themes;
        self.theme_before_preview = self.theme;
        self.theme_cursor = theme::index_of(self.theme.id);
    }

    fn confirm_overlay(&mut self, now: Instant) {
        if self.overlay == Overlay::Themes {
            let chosen = self.previewed_theme();
            self.theme = chosen;
            self.config.ui.theme = chosen.id.to_string();
            self.theme_before_preview = chosen;
            self.notify(format!("theme: {}", chosen.label), now);
        }
        self.overlay = Overlay::None;
    }

    fn cancel_overlay(&mut self) {
        if self.overlay == Overlay::Themes {
            self.theme = self.theme_before_preview;
            self.theme_cursor = theme::index_of(self.theme.id);
        }
        self.overlay = Overlay::None;
    }

    /// Halve or double the poll interval, within the configured bounds.
    fn scale_poll_interval(&mut self, factor: f64, now: Instant) -> Option<Command> {
        let current = self.poll_interval.as_millis() as f64;
        let wanted = (current * factor).round() as u64;
        let clamped = wanted.clamp(MIN_POLL_INTERVAL_MS, MAX_POLL_INTERVAL_MS);

        if clamped == self.poll_interval.as_millis() as u64 {
            self.notify(
                format!("poll interval already at its limit ({clamped}ms)"),
                now,
            );
            return None;
        }

        self.poll_interval = Duration::from_millis(clamped);
        self.config.ui.poll_interval_ms = clamped;
        // The retention window is expressed in seconds, so a faster poll must
        // buy more samples to keep covering the same span of time.
        self.snapshot
            .set_history_capacity(history_capacity(self.config.ui.history_seconds, clamped));
        self.notify(format!("poll interval: {clamped}ms"), now);
        Some(Command::SetPollInterval(self.poll_interval))
    }

    /// Rows in whatever list currently has the cursor.
    fn active_rows(&self) -> usize {
        if self.overlay == Overlay::Themes {
            theme::THEMES.len()
        } else if self.view.is_scrollable() {
            self.row_counts[self.view.index()]
        } else {
            0
        }
    }

    fn page_step(&self) -> usize {
        10
    }

    fn move_cursor(&mut self, delta: isize) {
        let rows = self.active_rows();
        if rows == 0 {
            return;
        }
        let current = if self.overlay == Overlay::Themes {
            self.theme_cursor
        } else {
            self.selection(self.view)
        } as isize;

        // Clamp rather than wrap: in a long log, wrapping from the newest
        // line to the oldest on one keypress is disorienting.
        let next = (current + delta).clamp(0, rows as isize - 1) as usize;
        self.write_cursor(next);
    }

    fn set_cursor(&mut self, position: usize) {
        let rows = self.active_rows();
        if rows == 0 {
            return;
        }
        self.write_cursor(position.min(rows - 1));
    }

    fn write_cursor(&mut self, position: usize) {
        if self.overlay == Overlay::Themes {
            self.theme_cursor = position;
            // Preview live, so the picker restyles the whole UI as it moves.
            self.theme = self.previewed_theme();
        } else {
            self.selection[self.view.index()] = position;
        }
    }

    fn clamp_selection(&mut self) {
        for (index, rows) in self.row_counts.iter().enumerate() {
            if *rows == 0 {
                self.selection[index] = 0;
            } else {
                self.selection[index] = self.selection[index].min(rows - 1);
            }
        }
        self.theme_cursor = self.theme_cursor.min(theme::THEMES.len() - 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::obs::client::{Inventory, PollData};
    use crate::obs::stats::{AudioInput, Stats, StreamStatus, Version};

    fn app() -> App {
        App::new(Config::default())
    }

    fn now() -> Instant {
        Instant::now()
    }

    #[test]
    fn starts_on_the_configured_view_and_theme() {
        let mut config = Config::default();
        config.ui.theme = "nord".into();
        config.ui.start_view = "audio".into();
        let app = App::new(config);
        assert_eq!(app.theme.id, "nord");
        assert_eq!(app.view, View::Audio);
        assert_eq!(app.theme_cursor, theme::index_of("nord"));
    }

    #[test]
    fn quitting_shuts_the_session_down() {
        let mut app = app();
        let command = app.handle(Action::Quit, now());
        assert!(app.should_quit);
        assert!(matches!(command, Some(Command::Shutdown)));
    }

    #[test]
    fn view_switching_wraps_and_jumps() {
        let mut app = app();
        assert_eq!(app.view, View::Overview);
        app.handle(Action::PreviousView, now());
        assert_eq!(app.view, View::Log);
        app.handle(Action::NextView, now());
        assert_eq!(app.view, View::Overview);
        app.handle(Action::SelectView(View::Stream), now());
        assert_eq!(app.view, View::Stream);
    }

    #[test]
    fn help_toggles_open_and_closed() {
        let mut app = app();
        app.handle(Action::ToggleHelp, now());
        assert_eq!(app.overlay, Overlay::Help);
        app.handle(Action::ToggleHelp, now());
        assert_eq!(app.overlay, Overlay::None);
    }

    #[test]
    fn cursor_moves_within_the_row_count_and_clamps() {
        let mut app = app();
        app.view = View::Scenes;
        app.set_row_count(View::Scenes, 3);

        app.handle(Action::Down, now());
        assert_eq!(app.selection(View::Scenes), 1);
        app.handle(Action::Down, now());
        app.handle(Action::Down, now());
        assert_eq!(app.selection(View::Scenes), 2, "must clamp, not wrap");
        app.handle(Action::Up, now());
        assert_eq!(app.selection(View::Scenes), 1);
        app.handle(Action::Up, now());
        app.handle(Action::Up, now());
        assert_eq!(app.selection(View::Scenes), 0, "must clamp at the top");
    }

    #[test]
    fn home_and_end_jump_to_the_extremes() {
        let mut app = app();
        app.view = View::Log;
        app.set_row_count(View::Log, 50);
        app.handle(Action::End, now());
        assert_eq!(app.selection(View::Log), 49);
        app.handle(Action::Home, now());
        assert_eq!(app.selection(View::Log), 0);
    }

    #[test]
    fn paging_moves_further_than_a_single_step() {
        let mut app = app();
        app.view = View::Log;
        app.set_row_count(View::Log, 100);
        app.handle(Action::PageDown, now());
        assert_eq!(app.selection(View::Log), 10);
        app.handle(Action::PageUp, now());
        assert_eq!(app.selection(View::Log), 0);
    }

    #[test]
    fn each_view_keeps_its_own_cursor() {
        let mut app = app();
        app.set_row_count(View::Scenes, 5);
        app.set_row_count(View::Audio, 5);

        app.view = View::Scenes;
        app.handle(Action::Down, now());
        app.handle(Action::Down, now());
        app.view = View::Audio;
        app.handle(Action::Down, now());

        assert_eq!(app.selection(View::Scenes), 2);
        assert_eq!(app.selection(View::Audio), 1);
    }

    #[test]
    fn cursor_keys_do_nothing_on_a_view_without_a_list() {
        let mut app = app();
        app.view = View::Overview;
        app.handle(Action::Down, now());
        assert_eq!(app.selection(View::Overview), 0);
    }

    #[test]
    fn a_shrinking_list_pulls_the_cursor_back_into_range() {
        let mut app = app();
        app.view = View::Scenes;
        app.set_row_count(View::Scenes, 10);
        app.handle(Action::End, now());
        assert_eq!(app.selection(View::Scenes), 9);

        // Scenes were deleted in OBS.
        app.set_row_count(View::Scenes, 3);
        assert_eq!(app.selection(View::Scenes), 2);

        app.set_row_count(View::Scenes, 0);
        assert_eq!(app.selection(View::Scenes), 0);
    }

    #[test]
    fn the_theme_picker_previews_live_and_commits_on_confirm() {
        let mut app = app();
        let original = app.theme.id;
        app.handle(Action::ToggleThemePicker, now());
        assert_eq!(app.overlay, Overlay::Themes);

        app.handle(Action::Down, now());
        assert_ne!(app.theme.id, original, "moving must preview immediately");
        let previewed = app.theme.id;

        app.handle(Action::Confirm, now());
        assert_eq!(app.overlay, Overlay::None);
        assert_eq!(app.theme.id, previewed);
        assert_eq!(app.config.ui.theme, previewed, "confirm updates the config");
    }

    #[test]
    fn cancelling_the_theme_picker_reverts_the_preview() {
        let mut app = app();
        let original = app.theme.id;
        app.handle(Action::ToggleThemePicker, now());
        app.handle(Action::Down, now());
        app.handle(Action::Down, now());
        assert_ne!(app.theme.id, original);

        app.handle(Action::Cancel, now());
        assert_eq!(app.overlay, Overlay::None);
        assert_eq!(app.theme.id, original, "cancel must restore the old theme");
        assert_eq!(app.config.ui.theme, original);
    }

    #[test]
    fn reopening_the_picker_starts_from_the_active_theme() {
        let mut app = app();
        app.handle(Action::ToggleThemePicker, now());
        app.handle(Action::End, now());
        app.handle(Action::Confirm, now());
        let chosen = app.theme.id;

        app.handle(Action::ToggleThemePicker, now());
        assert_eq!(app.theme_cursor, theme::index_of(chosen));
    }

    #[test]
    fn the_theme_cursor_cannot_run_past_the_theme_list() {
        let mut app = app();
        app.handle(Action::ToggleThemePicker, now());
        for _ in 0..(theme::THEMES.len() + 20) {
            app.handle(Action::Down, now());
        }
        assert_eq!(app.theme_cursor, theme::THEMES.len() - 1);
    }

    #[test]
    fn pausing_freezes_metrics_but_not_connection_state() {
        let mut app = app();
        let instant = now();
        app.handle(Action::TogglePause, instant);
        assert!(app.paused);

        app.apply(
            Update::Poll(Box::new(PollData {
                stats: Stats {
                    cpu_usage: 42.0,
                    ..Stats::default()
                },
                ..PollData::default()
            })),
            instant,
        );
        assert_eq!(app.snapshot.stats.cpu_usage, 0.0, "metrics must be frozen");

        // A dropped connection must still show through, or the dashboard lies.
        app.apply(
            Update::Disconnected {
                reason: "closed".into(),
                hint: None,
                retry_in: Duration::from_secs(1),
            },
            instant,
        );
        assert!(!app.snapshot.connection.is_connected());
    }

    #[test]
    fn refreshing_resumes_a_paused_dashboard() {
        let mut app = app();
        let instant = now();
        app.handle(Action::TogglePause, instant);
        assert!(app.paused);

        let command = app.handle(Action::Refresh, instant);
        assert!(
            !app.paused,
            "refresh must resume, or nothing visibly happens"
        );
        assert!(matches!(command, Some(Command::RefreshNow)));
    }

    #[test]
    fn poll_interval_halves_and_doubles_within_bounds() {
        let mut app = app();
        let instant = now();
        assert_eq!(app.poll_interval, Duration::from_millis(1000));

        let command = app.handle(Action::FasterPolling, instant);
        assert_eq!(app.poll_interval, Duration::from_millis(500));
        assert!(matches!(command, Some(Command::SetPollInterval(_))));

        app.handle(Action::SlowerPolling, instant);
        app.handle(Action::SlowerPolling, instant);
        assert_eq!(app.poll_interval, Duration::from_millis(2000));
        assert_eq!(app.config.ui.poll_interval_ms, 2000);
    }

    #[test]
    fn poll_interval_stops_at_its_limits_without_emitting_a_command() {
        let mut app = app();
        let instant = now();
        for _ in 0..20 {
            app.handle(Action::FasterPolling, instant);
        }
        assert_eq!(app.poll_interval.as_millis() as u64, MIN_POLL_INTERVAL_MS);
        // At the limit there is nothing to change, so no command is sent.
        assert!(app.handle(Action::FasterPolling, instant).is_none());

        for _ in 0..20 {
            app.handle(Action::SlowerPolling, instant);
        }
        assert_eq!(app.poll_interval.as_millis() as u64, MAX_POLL_INTERVAL_MS);
        assert!(app.handle(Action::SlowerPolling, instant).is_none());
    }

    #[test]
    fn changing_the_poll_rate_keeps_the_history_window_in_seconds() {
        let mut app = app();
        let instant = now();
        let before = app.snapshot.history.cpu.capacity();
        app.handle(Action::FasterPolling, instant);
        assert_eq!(
            app.snapshot.history.cpu.capacity(),
            before * 2,
            "twice the samples are needed to cover the same seconds"
        );
    }

    #[test]
    fn a_config_with_a_valid_but_unknown_theme_falls_back_rather_than_panicking() {
        let mut config = Config::default();
        config.ui.theme = "removed-in-a-later-release".into();
        let app = App::new(config);
        assert_eq!(app.theme.id, theme::FALLBACK.id);
    }

    #[test]
    fn toasts_expire_on_their_own() {
        let mut app = app();
        let instant = now();
        app.notify("hello".into(), instant);
        assert!(app.toast.is_some());

        app.tick(instant + Duration::from_millis(100));
        assert!(app.toast.is_some(), "still within the toast window");

        app.tick(instant + TOAST_DURATION + Duration::from_millis(1));
        assert!(app.toast.is_none());
    }

    #[test]
    fn unicode_drawing_toggles() {
        let mut app = app();
        assert!(app.unicode);
        app.handle(Action::ToggleUnicode, now());
        assert!(!app.unicode);
    }

    #[test]
    fn updates_flow_into_the_snapshot_when_not_paused() {
        let mut app = app();
        let instant = now();
        app.apply(
            Update::Connected(Box::new(Version {
                obs_version: "30.2.3".into(),
                ..Version::default()
            })),
            instant,
        );
        assert!(app.snapshot.connection.is_connected());
        assert_eq!(app.snapshot.version.obs_version, "30.2.3");

        app.apply(
            Update::Inventory(Box::new(Inventory {
                inputs: vec![AudioInput {
                    name: "Mic".into(),
                    ..AudioInput::default()
                }],
                ..Inventory::default()
            })),
            instant,
        );
        assert_eq!(app.snapshot.inputs.len(), 1);

        app.apply(
            Update::Poll(Box::new(PollData {
                stream: StreamStatus {
                    active: true,
                    ..StreamStatus::default()
                },
                ..PollData::default()
            })),
            instant,
        );
        assert!(app.snapshot.stream.active);
    }

    #[test]
    fn the_frame_counter_advances_on_every_tick() {
        let mut app = app();
        let instant = now();
        app.tick(instant);
        app.tick(instant);
        assert_eq!(app.frame, 2);
    }
}
