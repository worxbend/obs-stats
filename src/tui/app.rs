//! Application state and the action handler.
//!
//! `App` holds everything the renderer needs and nothing else — no terminal,
//! no socket, no channels. [`App::handle`] is a pure state transition that
//! returns the OBS command (if any) the shell should forward, which is what
//! lets the entire interaction model be tested without a TTY.
//!
//! It also holds the geometry the last frame drew — [`Regions`] — so a mouse
//! click can be mapped back to whatever is under it. Recording where something
//! was drawn is not I/O, and keeping it here rather than in the shell means
//! hit-testing is testable without a terminal too.

use std::time::{Duration, Instant};

use ratatui::layout::Rect;
use ratatui::widgets::ListState;

use crate::config::Config;
use crate::config::model::{MAX_POLL_INTERVAL_MS, MIN_POLL_INTERVAL_MS};
use crate::metrics::alerts::{Alert, Severity};
use crate::metrics::{Snapshot, history_capacity};
use crate::obs::client::{Command, Update};
use crate::tui::command::{self, PaletteItem};
use crate::tui::keys::{Action, Overlay, Pending};
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

/// Rows a list pages by when its rendered height is not yet known.
const DEFAULT_PAGE: usize = 10;

/// An alert currently shown as a banner.
#[derive(Debug, Clone, PartialEq)]
pub struct Banner {
    pub alert: Alert,
    pub until: Instant,
}

/// Something in the chrome a click can land on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    Pause,
    Poll,
    ThemeName,
    Alerts,
}

/// Where the last frame drew the things a mouse can reach.
///
/// Every field is reset to `None` each frame and repopulated by whatever
/// actually drew, so a click can never hit a widget that is no longer there —
/// a stale rect is the one bug this whole struct exists to avoid.
#[derive(Debug, Default, Clone)]
pub struct Regions {
    /// The tab strip.
    pub tabs: Rect,
    /// The active view's body.
    pub body: Rect,
    /// The focused view's list, if it drew one.
    pub list: Option<Rect>,
    /// The list's scrollbar track, when the list is long enough to have one.
    pub scrollbar: Option<Rect>,
    /// The row list inside whichever overlay is open.
    pub overlay_list: Option<Rect>,
    /// The overlay's whole panel, so a click outside it can dismiss it.
    pub overlay_panel: Option<Rect>,
    /// Clickable cells in the header and status bar.
    pub hits: Vec<(Rect, Hit)>,
}

impl Regions {
    /// Clear everything positional, keeping the allocation.
    fn clear(&mut self) {
        self.list = None;
        self.scrollbar = None;
        self.overlay_list = None;
        self.overlay_panel = None;
        self.hits.clear();
    }

    /// The chrome target at `(column, row)`, if any.
    pub fn hit(&self, column: u16, row: u16) -> Option<Hit> {
        self.hits
            .iter()
            .find(|(rect, _)| contains(*rect, column, row))
            .map(|(_, hit)| *hit)
    }
}

/// Whether `rect` covers the cell at `(column, row)`.
///
/// `Rect::contains` needs a `Position`; this keeps the call sites readable and
/// gives the zero-sized case an explicit answer.
pub fn contains(rect: Rect, column: u16, row: u16) -> bool {
    rect.width > 0
        && rect.height > 0
        && column >= rect.x
        && column < rect.x.saturating_add(rect.width)
        && row >= rect.y
        && row < rect.y.saturating_add(rect.height)
}

/// The `:` command line.
#[derive(Debug, Default, Clone)]
pub struct CommandLine {
    /// What has been typed, without the leading `:`.
    pub input: String,
    /// Lines already run, oldest first.
    pub history: Vec<String>,
    /// Position while walking history; `None` means "on the fresh line".
    history_index: Option<usize>,
    /// The completion cycle for the current stem.
    completions: Vec<String>,
    completion_index: Option<usize>,
}

impl CommandLine {
    fn reset(&mut self) {
        self.input.clear();
        self.history_index = None;
        self.clear_completions();
    }

    fn clear_completions(&mut self) {
        self.completions.clear();
        self.completion_index = None;
    }

    /// Step through the completions for what is typed, wrapping both ways.
    fn complete(&mut self, forward: bool) {
        if self.completions.is_empty() {
            self.completions = command::complete(&self.input);
            self.completion_index = None;
            if self.completions.is_empty() {
                return;
            }
        }

        let count = self.completions.len();
        let next = match (self.completion_index, forward) {
            (None, true) => 0,
            (None, false) => count - 1,
            (Some(current), true) => (current + 1) % count,
            (Some(current), false) => (current + count - 1) % count,
        };
        self.completion_index = Some(next);
        self.input = self.completions[next].clone();
    }

    /// Record a line that was run, most recent last, without adjacent repeats.
    fn remember(&mut self, line: &str) {
        if line.is_empty() {
            return;
        }
        if self.history.last().map(String::as_str) == Some(line) {
            return;
        }
        self.history.push(line.to_string());
    }

    /// Walk backwards (`older`) or forwards through the history.
    fn recall(&mut self, older: bool) {
        if self.history.is_empty() {
            return;
        }
        let last = self.history.len() - 1;
        self.history_index = match (self.history_index, older) {
            (None, true) => Some(last),
            (None, false) => None,
            (Some(0), true) => Some(0),
            (Some(current), true) => Some(current - 1),
            (Some(current), false) if current >= last => None,
            (Some(current), false) => Some(current + 1),
        };
        self.input = match self.history_index {
            Some(index) => self.history[index].clone(),
            None => String::new(),
        };
        self.clear_completions();
    }
}

/// The fuzzy command palette.
#[derive(Debug, Default, Clone)]
pub struct Palette {
    /// What has been typed.
    pub query: String,
    /// Cursor into [`Palette::matches`].
    pub cursor: usize,
    /// Restrict rows to one kind — `theme`, `view` — or all when empty.
    pub filter: &'static str,
    /// The rows currently shown, best match first.
    pub matches: Vec<PaletteItem>,
    /// Every row, computed once when the palette opens.
    source: Vec<PaletteItem>,
}

impl Palette {
    fn open(&mut self, filter: &'static str) {
        self.query.clear();
        self.cursor = 0;
        self.filter = filter;
        self.source = command::palette_items();
        if !filter.is_empty() {
            self.source.retain(|item| item.kind == filter);
        }
        self.refilter();
    }

    fn refilter(&mut self) {
        self.matches = command::filter(&self.source, &self.query);
        self.cursor = self.cursor.min(self.matches.len().saturating_sub(1));
    }

    /// The row under the cursor.
    pub fn selected(&self) -> Option<&PaletteItem> {
        self.matches.get(self.cursor)
    }

    /// What to print above the list.
    pub fn title(&self) -> &'static str {
        match self.filter {
            "theme" => "Themes",
            "view" => "Views",
            _ => "Commands",
        }
    }
}

pub struct App {
    pub config: Config,
    pub snapshot: Snapshot,
    pub theme: &'static Theme,
    pub view: View,
    pub overlay: Overlay,
    /// A key sequence part-way through being typed, e.g. `<leader>u`.
    pub pending: Pending,
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
    /// How many rows of each list were actually on screen, for paging and for
    /// mapping a click to a row.
    visible_rows: [usize; View::ALL.len()],
    /// Scroll state per view, owned here so the rendered offset survives the
    /// frame and a click can be resolved against it.
    list_states: [ListState; View::ALL.len()],
    /// The same, for whichever overlay is open — only ever one at a time.
    overlay_list_state: ListState,
    /// Cursor in the theme picker.
    pub theme_cursor: usize,
    /// Theme to restore if the picker is cancelled.
    theme_before_preview: &'static Theme,
    pub command: CommandLine,
    pub palette: Palette,
    /// Where the last frame drew everything clickable.
    pub regions: Regions,
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
            pending: Pending::None,
            paused: false,
            unicode: config.ui.unicode,
            show_icons: config.ui.show_icons,
            poll_interval: Duration::from_millis(config.ui.poll_interval_ms),
            should_quit: false,
            frame: 0,
            version: crate::VERSION.to_string(),
            selection: [0; View::ALL.len()],
            row_counts: [0; View::ALL.len()],
            visible_rows: [0; View::ALL.len()],
            list_states: Default::default(),
            overlay_list_state: ListState::default(),
            theme_cursor: theme::index_of(&config.ui.theme),
            theme_before_preview: theme,
            command: CommandLine::default(),
            palette: Palette::default(),
            regions: Regions::default(),
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
                Update::Poll(_) | Update::Inventory(_) | Update::Meters(_) | Update::Faders(_)
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
            Action::HalfPageUp => {
                self.move_cursor(-((self.page_step() / 2).max(1) as isize));
                None
            }
            Action::HalfPageDown => {
                self.move_cursor((self.page_step() / 2).max(1) as isize);
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
            Action::SetPollInterval(ms) => self.set_poll_interval(ms, now),
            Action::ToggleUnicode => {
                self.unicode = !self.unicode;
                self.config.ui.unicode = self.unicode;
                None
            }
            Action::ToggleIcons => {
                self.show_icons = !self.show_icons;
                self.config.ui.show_icons = self.show_icons;
                None
            }
            Action::SetTheme(theme) => {
                self.theme = theme;
                self.theme_before_preview = theme;
                self.theme_cursor = theme::index_of(theme.id);
                self.config.ui.theme = theme.id.to_string();
                self.notify(format!("theme: {}", theme.label), now);
                None
            }
            Action::OpenCommandLine => {
                self.overlay = Overlay::Command;
                self.command.reset();
                None
            }
            Action::OpenPalette => {
                self.overlay = Overlay::Palette;
                self.palette.open("");
                self.overlay_list_state = ListState::default();
                None
            }
            Action::OpenPaletteFiltered(kind) => {
                self.overlay = Overlay::Palette;
                self.palette.open(kind);
                self.overlay_list_state = ListState::default();
                None
            }
            Action::Input(c) => {
                self.input(c);
                None
            }
            Action::Backspace => {
                self.edit_input(|input| {
                    input.pop();
                });
                None
            }
            Action::DeleteWordBack => {
                self.edit_input(|input| {
                    while input.ends_with(' ') {
                        input.pop();
                    }
                    while !input.is_empty() && !input.ends_with(' ') {
                        input.pop();
                    }
                });
                None
            }
            Action::ClearLine => {
                self.edit_input(String::clear);
                None
            }
            Action::CompleteNext => {
                if self.overlay == Overlay::Command {
                    self.command.complete(true);
                }
                None
            }
            Action::CompletePrevious => {
                if self.overlay == Overlay::Command {
                    self.command.complete(false);
                }
                None
            }
            Action::HistoryPrevious => {
                if self.overlay == Overlay::Command {
                    self.command.recall(true);
                }
                None
            }
            Action::HistoryNext => {
                if self.overlay == Overlay::Command {
                    self.command.recall(false);
                }
                None
            }
            Action::Confirm => self.confirm_overlay(now),
            Action::Cancel => {
                self.cancel_overlay();
                None
            }
        }
    }

    /// Feed one typed character to whichever text input is open.
    fn input(&mut self, c: char) {
        match self.overlay {
            Overlay::Command => {
                self.command.input.push(c);
                self.command.clear_completions();
                self.command.history_index = None;
            }
            Overlay::Palette => {
                self.palette.query.push(c);
                self.palette.cursor = 0;
                self.palette.refilter();
            }
            _ => {}
        }
    }

    /// Apply `edit` to whichever text input is open, then resync it.
    fn edit_input(&mut self, edit: impl FnOnce(&mut String)) {
        match self.overlay {
            Overlay::Command => {
                edit(&mut self.command.input);
                self.command.clear_completions();
            }
            Overlay::Palette => {
                edit(&mut self.palette.query);
                self.palette.cursor = 0;
                self.palette.refilter();
            }
            _ => {}
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

    /// Rows in `view`'s list, as of the last frame.
    pub fn row_count(&self, view: View) -> usize {
        self.row_counts[view.index()]
    }

    /// Borrow `view`'s scroll state so a list widget can advance it.
    ///
    /// Taken out and handed back rather than borrowed in place: the views hold
    /// a shared borrow of the snapshot while they build their rows, which a
    /// simultaneous `&mut self` would conflict with.
    pub fn take_list_state(&mut self, view: View) -> ListState {
        std::mem::take(&mut self.list_states[view.index()])
    }

    /// Hand back the state from [`App::take_list_state`], recording where the
    /// list was drawn so a click can be resolved against it.
    pub fn put_list_state(&mut self, view: View, area: Rect, state: ListState) {
        self.visible_rows[view.index()] = area.height as usize;
        if view == self.view {
            self.regions.list = Some(area);
        }
        self.list_states[view.index()] = state;
    }

    /// The first row of `view`'s list that was on screen last frame.
    pub fn list_offset(&self, view: View) -> usize {
        self.list_states[view.index()].offset()
    }

    /// Borrow the open overlay's scroll state, as [`App::take_list_state`].
    pub fn take_overlay_list_state(&mut self) -> ListState {
        std::mem::take(&mut self.overlay_list_state)
    }

    /// Hand it back, recording where the overlay and its rows were drawn.
    pub fn put_overlay_list_state(&mut self, panel: Rect, list: Rect, state: ListState) {
        self.regions.overlay_panel = Some(panel);
        self.regions.overlay_list = Some(list);
        self.overlay_list_state = state;
    }

    /// The first overlay row that was on screen last frame.
    pub fn overlay_offset(&self) -> usize {
        self.overlay_list_state.offset()
    }

    /// Move the cursor to the row at terminal `row`, if a list is under it.
    ///
    /// Returns whether anything moved, so a click that lands on a list but
    /// past its last row is not mistaken for a miss.
    pub fn click_row(&mut self, row: u16) -> bool {
        let Some(area) = self.regions.list else {
            return false;
        };
        if row < area.y || row >= area.y + area.height {
            return false;
        }
        let index = self.list_offset(self.view) + (row - area.y) as usize;
        if index >= self.row_counts[self.view.index()] {
            return false;
        }
        self.selection[self.view.index()] = index;
        true
    }

    /// Put the active view's cursor on `index`, clamped to the list.
    ///
    /// Used by the scrollbar, where the target row comes from geometry rather
    /// than from a relative move.
    pub fn select_row(&mut self, index: usize) {
        let rows = self.row_counts[self.view.index()];
        if rows == 0 {
            return;
        }
        self.selection[self.view.index()] = index.min(rows - 1);
    }

    /// Move the overlay's cursor to the row at terminal `row`.
    pub fn click_overlay_row(&mut self, row: u16) -> bool {
        let Some(area) = self.regions.overlay_list else {
            return false;
        };
        if row < area.y || row >= area.y + area.height {
            return false;
        }
        let index = self.overlay_offset() + (row - area.y) as usize;

        match self.overlay {
            Overlay::Themes => {
                if index >= theme::THEMES.len() {
                    return false;
                }
                self.theme_cursor = index;
                self.theme = self.previewed_theme();
                true
            }
            Overlay::Palette => {
                if index >= self.palette.matches.len() {
                    return false;
                }
                self.palette.cursor = index;
                true
            }
            _ => false,
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

    /// Drop last frame's geometry. Called at the top of every draw, so a
    /// widget that stops drawing stops being clickable in the same frame.
    pub fn begin_frame(&mut self) {
        self.regions.clear();
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
        // A scroll position left over from the last overlay would put the
        // rows somewhere other than where a click expects them.
        self.overlay_list_state = ListState::default();
    }

    /// Commit whatever the open overlay was for.
    fn confirm_overlay(&mut self, now: Instant) -> Option<Command> {
        match self.overlay {
            Overlay::Themes => {
                let chosen = self.previewed_theme();
                self.theme = chosen;
                self.config.ui.theme = chosen.id.to_string();
                self.theme_before_preview = chosen;
                self.notify(format!("theme: {}", chosen.label), now);
                self.overlay = Overlay::None;
                None
            }
            Overlay::Command => {
                let line = self.command.input.trim().to_string();
                self.command.remember(&line);
                self.overlay = Overlay::None;
                match command::parse(&line) {
                    Ok(action) => self.handle(action, now),
                    Err(problem) => {
                        self.notify(problem, now);
                        None
                    }
                }
            }
            Overlay::Palette => {
                let chosen = self.palette.selected().map(|item| item.action);
                self.overlay = Overlay::None;
                match chosen {
                    Some(action) => self.handle(action, now),
                    None => None,
                }
            }
            _ => {
                self.overlay = Overlay::None;
                None
            }
        }
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
        self.set_poll_interval(wanted, now)
    }

    /// Set the poll cadence to `wanted` ms, clamped to the supported range.
    fn set_poll_interval(&mut self, wanted: u64, now: Instant) -> Option<Command> {
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
        match self.overlay {
            Overlay::Themes => theme::THEMES.len(),
            Overlay::Palette => self.palette.matches.len(),
            _ if self.view.is_scrollable() => self.row_counts[self.view.index()],
            _ => 0,
        }
    }

    /// How far a page key moves, taken from what was actually on screen.
    fn page_step(&self) -> usize {
        let rows = self.visible_rows[self.view.index()];
        if self.overlay == Overlay::None && rows > 1 {
            rows - 1
        } else {
            DEFAULT_PAGE
        }
    }

    fn move_cursor(&mut self, delta: isize) {
        let rows = self.active_rows();
        if rows == 0 {
            return;
        }
        let current = self.cursor() as isize;

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

    /// The cursor of whichever list currently has focus.
    fn cursor(&self) -> usize {
        match self.overlay {
            Overlay::Themes => self.theme_cursor,
            Overlay::Palette => self.palette.cursor,
            _ => self.selection(self.view),
        }
    }

    fn write_cursor(&mut self, position: usize) {
        match self.overlay {
            Overlay::Themes => {
                self.theme_cursor = position;
                // Preview live, so the picker restyles the whole UI as it moves.
                self.theme = self.previewed_theme();
            }
            Overlay::Palette => self.palette.cursor = position,
            _ => self.selection[self.view.index()] = position,
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

    /// Type a line into whichever text input is open.
    fn type_line(app: &mut App, text: &str) {
        for c in text.chars() {
            app.handle(Action::Input(c), now());
        }
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
        assert_eq!(app.pending, Pending::None);
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
        assert_eq!(app.selection(View::Log), DEFAULT_PAGE);
        app.handle(Action::PageUp, now());
        assert_eq!(app.selection(View::Log), 0);
    }

    #[test]
    fn paging_uses_the_height_the_list_actually_drew() {
        let mut app = app();
        app.view = View::Log;
        app.set_row_count(View::Log, 200);
        // 20 rows on screen: a page is one row shy of a screenful, so the
        // line you were reading stays visible.
        app.put_list_state(View::Log, Rect::new(0, 0, 40, 20), ListState::default());

        app.handle(Action::PageDown, now());
        assert_eq!(app.selection(View::Log), 19);
        app.handle(Action::HalfPageDown, now());
        assert_eq!(app.selection(View::Log), 19 + 9);
        app.handle(Action::HalfPageUp, now());
        assert_eq!(app.selection(View::Log), 19);
    }

    #[test]
    fn half_paging_always_moves_at_least_one_row() {
        let mut app = app();
        app.view = View::Log;
        app.set_row_count(View::Log, 20);
        // A two-row list would give a half-page of zero without the floor,
        // leaving Ctrl-d silently inert.
        app.put_list_state(View::Log, Rect::new(0, 0, 40, 2), ListState::default());
        app.handle(Action::HalfPageDown, now());
        assert_eq!(app.selection(View::Log), 1);
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
    fn setting_the_poll_interval_outright_behaves_like_scaling_it() {
        let mut app = app();
        let instant = now();
        let command = app.handle(Action::SetPollInterval(250), instant);
        assert_eq!(app.poll_interval, Duration::from_millis(250));
        assert_eq!(app.config.ui.poll_interval_ms, 250);
        assert!(matches!(command, Some(Command::SetPollInterval(_))));
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
    fn unicode_and_icon_toggles_write_through_to_the_config() {
        let mut app = app();
        assert!(app.unicode);
        app.handle(Action::ToggleUnicode, now());
        assert!(!app.unicode);
        assert!(!app.config.ui.unicode);

        let icons = app.show_icons;
        app.handle(Action::ToggleIcons, now());
        assert_eq!(app.show_icons, !icons);
        assert_eq!(app.config.ui.show_icons, !icons);
    }

    #[test]
    fn setting_a_theme_by_action_applies_it_and_survives_a_later_cancel() {
        let mut app = app();
        let nord = theme::find("nord").unwrap();
        app.handle(Action::SetTheme(nord), now());
        assert_eq!(app.theme.id, "nord");
        assert_eq!(app.config.ui.theme, "nord");

        // Opening and cancelling the picker must come back to nord, not to
        // whatever was set before the command.
        app.handle(Action::ToggleThemePicker, now());
        app.handle(Action::Down, now());
        app.handle(Action::Cancel, now());
        assert_eq!(app.theme.id, "nord");
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

    // ---- the command line -------------------------------------------------

    #[test]
    fn the_command_line_runs_what_was_typed() {
        let mut app = app();
        app.handle(Action::OpenCommandLine, now());
        assert_eq!(app.overlay, Overlay::Command);

        type_line(&mut app, "view audio");
        app.handle(Action::Confirm, now());

        assert_eq!(app.overlay, Overlay::None, "running closes the line");
        assert_eq!(app.view, View::Audio);
    }

    #[test]
    fn a_command_that_produces_an_obs_command_forwards_it() {
        let mut app = app();
        app.handle(Action::OpenCommandLine, now());
        type_line(&mut app, "poll 250");
        let command = app.handle(Action::Confirm, now());
        assert!(matches!(command, Some(Command::SetPollInterval(_))));
        assert_eq!(app.poll_interval, Duration::from_millis(250));
    }

    #[test]
    fn quitting_from_the_command_line_still_shuts_down() {
        let mut app = app();
        app.handle(Action::OpenCommandLine, now());
        type_line(&mut app, "q");
        let command = app.handle(Action::Confirm, now());
        assert!(app.should_quit);
        assert!(matches!(command, Some(Command::Shutdown)));
    }

    #[test]
    fn a_bad_command_reports_itself_without_closing_anything_else() {
        let mut app = app();
        app.handle(Action::OpenCommandLine, now());
        type_line(&mut app, "wibble");
        app.handle(Action::Confirm, now());

        assert_eq!(app.overlay, Overlay::None);
        assert_eq!(
            app.toast.as_ref().map(|toast| toast.message.as_str()),
            Some("not a command: wibble")
        );
    }

    #[test]
    fn cancelling_the_command_line_runs_nothing() {
        let mut app = app();
        app.handle(Action::OpenCommandLine, now());
        type_line(&mut app, "quit");
        app.handle(Action::Cancel, now());
        assert_eq!(app.overlay, Overlay::None);
        assert!(!app.should_quit);
    }

    #[test]
    fn editing_keys_rub_out_characters_words_and_the_whole_line() {
        let mut app = app();
        app.handle(Action::OpenCommandLine, now());
        type_line(&mut app, "theme nord");

        app.handle(Action::Backspace, now());
        assert_eq!(app.command.input, "theme nor");

        app.handle(Action::DeleteWordBack, now());
        assert_eq!(app.command.input, "theme ");

        app.handle(Action::ClearLine, now());
        assert!(app.command.input.is_empty());
    }

    #[test]
    fn tab_cycles_completions_in_both_directions() {
        let mut app = app();
        app.handle(Action::OpenCommandLine, now());
        type_line(&mut app, "theme ");

        app.handle(Action::CompleteNext, now());
        let first = app.command.input.clone();
        app.handle(Action::CompleteNext, now());
        let second = app.command.input.clone();
        assert_ne!(first, second);

        app.handle(Action::CompletePrevious, now());
        assert_eq!(app.command.input, first, "Shift-Tab walks back");
    }

    #[test]
    fn completing_an_unambiguous_prefix_lands_on_the_only_answer() {
        let mut app = app();
        app.handle(Action::OpenCommandLine, now());
        type_line(&mut app, "th");
        app.handle(Action::CompleteNext, now());
        assert_eq!(app.command.input, "theme ");
    }

    #[test]
    fn typing_after_a_completion_starts_a_fresh_cycle() {
        let mut app = app();
        app.handle(Action::OpenCommandLine, now());
        type_line(&mut app, "the");
        app.handle(Action::CompleteNext, now());
        assert_eq!(app.command.input, "theme ");

        type_line(&mut app, "nor");
        app.handle(Action::CompleteNext, now());
        assert_eq!(app.command.input, "theme nord");
    }

    #[test]
    fn history_walks_back_through_what_was_run() {
        let mut app = app();
        for line in ["view log", "view audio"] {
            app.handle(Action::OpenCommandLine, now());
            type_line(&mut app, line);
            app.handle(Action::Confirm, now());
        }

        app.handle(Action::OpenCommandLine, now());
        app.handle(Action::HistoryPrevious, now());
        assert_eq!(app.command.input, "view audio", "newest first");
        app.handle(Action::HistoryPrevious, now());
        assert_eq!(app.command.input, "view log");
        app.handle(Action::HistoryPrevious, now());
        assert_eq!(app.command.input, "view log", "clamps at the oldest");

        app.handle(Action::HistoryNext, now());
        assert_eq!(app.command.input, "view audio");
        app.handle(Action::HistoryNext, now());
        assert!(app.command.input.is_empty(), "back to the fresh line");
    }

    #[test]
    fn history_skips_blanks_and_immediate_repeats() {
        let mut app = app();
        for line in ["view log", "view log", ""] {
            app.handle(Action::OpenCommandLine, now());
            type_line(&mut app, line);
            app.handle(Action::Confirm, now());
        }
        assert_eq!(app.command.history, vec!["view log".to_string()]);
    }

    // ---- the palette ------------------------------------------------------

    #[test]
    fn the_palette_filters_as_you_type_and_runs_the_pick() {
        let mut app = app();
        app.handle(Action::OpenPalette, now());
        assert_eq!(app.overlay, Overlay::Palette);
        let all = app.palette.matches.len();

        type_line(&mut app, "audio");
        assert!(app.palette.matches.len() < all);
        assert_eq!(app.palette.matches[0].label, "view audio");

        app.handle(Action::Confirm, now());
        assert_eq!(app.overlay, Overlay::None);
        assert_eq!(app.view, View::Audio);
    }

    #[test]
    fn a_filtered_palette_only_offers_its_own_kind() {
        let mut app = app();
        app.handle(Action::OpenPaletteFiltered("theme"), now());
        assert_eq!(app.palette.matches.len(), theme::THEMES.len());
        assert!(app.palette.matches.iter().all(|item| item.kind == "theme"));
        assert_eq!(app.palette.title(), "Themes");

        type_line(&mut app, "nord");
        app.handle(Action::Confirm, now());
        assert_eq!(app.theme.id, "nord");
    }

    #[test]
    fn the_palette_cursor_moves_and_clamps_to_the_matches() {
        let mut app = app();
        app.handle(Action::OpenPaletteFiltered("view"), now());
        assert_eq!(app.palette.matches.len(), View::ALL.len());

        app.handle(Action::Down, now());
        assert_eq!(app.palette.cursor, 1);
        for _ in 0..50 {
            app.handle(Action::Down, now());
        }
        assert_eq!(app.palette.cursor, View::ALL.len() - 1, "clamps at the end");

        app.handle(Action::Home, now());
        assert_eq!(app.palette.cursor, 0);
    }

    #[test]
    fn narrowing_the_query_pulls_the_cursor_back_into_range() {
        let mut app = app();
        app.handle(Action::OpenPaletteFiltered("theme"), now());
        app.handle(Action::End, now());
        assert!(app.palette.cursor > 0);

        type_line(&mut app, "nord");
        assert!(
            app.palette.cursor < app.palette.matches.len(),
            "a cursor past the last match would confirm nothing"
        );
    }

    #[test]
    fn confirming_a_palette_with_no_matches_does_nothing() {
        let mut app = app();
        app.handle(Action::OpenPalette, now());
        type_line(&mut app, "zzzzzz");
        assert!(app.palette.matches.is_empty());

        let command = app.handle(Action::Confirm, now());
        assert!(command.is_none());
        assert_eq!(app.overlay, Overlay::None);
        assert!(!app.should_quit);
    }

    #[test]
    fn backspacing_in_the_palette_widens_the_matches_again() {
        let mut app = app();
        app.handle(Action::OpenPalette, now());
        let everything = app.palette.matches.len();

        type_line(&mut app, "nord");
        assert!(app.palette.matches.len() < everything);

        for _ in 0..4 {
            app.handle(Action::Backspace, now());
        }
        assert_eq!(
            app.palette.matches.len(),
            everything,
            "clearing the query must offer everything again"
        );
    }

    // ---- geometry and clicks ---------------------------------------------

    #[test]
    fn a_click_lands_on_the_row_under_it() {
        let mut app = app();
        app.view = View::Log;
        app.set_row_count(View::Log, 40);
        app.put_list_state(View::Log, Rect::new(2, 5, 40, 10), ListState::default());

        assert!(app.click_row(5), "the first row of the list");
        assert_eq!(app.selection(View::Log), 0);
        assert!(app.click_row(9));
        assert_eq!(app.selection(View::Log), 4);
    }

    #[test]
    fn a_click_accounts_for_how_far_the_list_is_scrolled() {
        let mut app = app();
        app.view = View::Log;
        app.set_row_count(View::Log, 100);
        app.put_list_state(
            View::Log,
            Rect::new(0, 4, 40, 10),
            ListState::default().with_offset(30),
        );

        assert!(app.click_row(4));
        assert_eq!(
            app.selection(View::Log),
            30,
            "the top row is the 31st entry, not the first"
        );
    }

    #[test]
    fn clicks_outside_the_list_or_past_its_last_row_miss() {
        let mut app = app();
        app.view = View::Log;
        app.set_row_count(View::Log, 3);
        app.put_list_state(View::Log, Rect::new(0, 5, 40, 10), ListState::default());

        assert!(!app.click_row(4), "above the list");
        assert!(!app.click_row(20), "below the list");
        // Rows 5,6,7 hold the three entries; 8 is empty space inside the panel.
        assert!(app.click_row(7));
        assert!(!app.click_row(8), "past the last entry");
        assert_eq!(app.selection(View::Log), 2);
    }

    #[test]
    fn a_view_with_no_list_this_frame_cannot_be_clicked() {
        let mut app = app();
        app.view = View::Overview;
        app.begin_frame();
        assert!(!app.click_row(5));
    }

    #[test]
    fn beginning_a_frame_forgets_where_everything_was() {
        let mut app = app();
        app.view = View::Log;
        app.set_row_count(View::Log, 10);
        app.put_list_state(View::Log, Rect::new(0, 0, 10, 10), ListState::default());
        app.regions.hits.push((Rect::new(0, 0, 4, 1), Hit::Pause));
        assert!(app.regions.list.is_some());

        app.begin_frame();
        assert!(app.regions.list.is_none(), "a stale rect is a wrong click");
        assert!(app.regions.hits.is_empty());
    }

    #[test]
    fn clicking_a_theme_row_previews_it_and_a_second_click_confirms() {
        let mut app = app();
        app.handle(Action::ToggleThemePicker, now());
        app.regions.overlay_list = Some(Rect::new(0, 3, 40, 12));

        let before = app.theme.id;
        assert!(app.click_overlay_row(5));
        assert_eq!(app.theme_cursor, 2);
        assert_ne!(app.theme.id, before, "clicking previews live");

        app.handle(Action::Confirm, now());
        assert_eq!(app.overlay, Overlay::None);
        assert_eq!(app.theme.id, theme::THEMES[2].id);
    }

    #[test]
    fn clicking_a_palette_row_selects_it() {
        let mut app = app();
        app.handle(Action::OpenPaletteFiltered("view"), now());
        app.regions.overlay_list = Some(Rect::new(0, 2, 40, 10));

        assert!(app.click_overlay_row(4));
        assert_eq!(app.palette.cursor, 2);
        app.handle(Action::Confirm, now());
        assert_eq!(app.view, View::ALL[2]);
    }

    #[test]
    fn clicking_an_empty_overlay_row_changes_nothing() {
        let mut app = app();
        app.handle(Action::OpenPaletteFiltered("view"), now());
        app.regions.overlay_list = Some(Rect::new(0, 0, 40, 20));
        // Only six views, so row 10 is blank space inside the panel.
        assert!(!app.click_overlay_row(10));
        assert_eq!(app.palette.cursor, 0);
    }

    #[test]
    fn chrome_hit_testing_finds_the_cell_under_the_pointer() {
        let mut regions = Regions::default();
        regions.hits.push((Rect::new(10, 20, 6, 1), Hit::Pause));
        regions.hits.push((Rect::new(20, 20, 6, 1), Hit::Poll));

        assert_eq!(regions.hit(10, 20), Some(Hit::Pause));
        assert_eq!(regions.hit(15, 20), Some(Hit::Pause));
        assert_eq!(regions.hit(16, 20), None, "the gap between them");
        assert_eq!(regions.hit(22, 20), Some(Hit::Poll));
        assert_eq!(regions.hit(10, 21), None, "the wrong row");
    }

    #[test]
    fn a_zero_sized_rect_contains_nothing() {
        assert!(!contains(Rect::new(5, 5, 0, 1), 5, 5));
        assert!(!contains(Rect::new(5, 5, 1, 0), 5, 5));
        assert!(contains(Rect::new(5, 5, 1, 1), 5, 5));
        assert!(!contains(Rect::new(5, 5, 1, 1), 6, 5));
    }
}
