//! Key bindings, in the shape AstroNvim users already have in their fingers.
//!
//! Resolution is a pure function of `(overlay, pending, key)` → [`Resolution`],
//! with no access to app state. Multi-key sequences — `<leader>uu`, `gg`, `]b`
//! — are expressed by threading the pending prefix *in* rather than by holding
//! it here, which keeps the whole keymap testable without a terminal and lets
//! the which-key popup and the help overlay both be generated from the same
//! [`LEADER_MAP`] and [`BINDINGS`] tables that document the app.
//!
//! Every key the app shipped with before the vim layer still works. The one
//! exception is bare `g`, which is now the `gg` prefix rather than "jump to
//! top" — `Home` covers that unchanged.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::tui::theme::Theme;
use crate::tui::view::View;

/// The leader key, printed wherever a binding is described.
pub const LEADER_LABEL: &str = "space";

/// Which modal layer currently owns the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Overlay {
    /// The dashboard itself.
    #[default]
    None,
    /// The key-binding reference.
    Help,
    /// The live theme picker.
    Themes,
    /// The `:` command line.
    Command,
    /// The fuzzy command palette.
    Palette,
}

/// A key sequence part-way through being typed.
///
/// Held by the app rather than by this module, so resolution stays pure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Pending {
    /// Nothing outstanding.
    #[default]
    None,
    /// `<leader>` was pressed.
    Leader,
    /// `<leader>` then a group key, e.g. `<leader>f`.
    LeaderGroup(char),
    /// A vim prefix that is not the leader: `g`, `[` or `]`.
    Prefix(char),
}

/// What a key press meant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Resolution {
    /// Do this.
    Act(Action),
    /// The key started or extended a sequence; wait for the next one.
    Wait(Pending),
    /// The key has no meaning here. Any pending sequence is abandoned.
    Ignore,
}

/// Something the app should do in response to a key.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    Quit,
    ToggleHelp,
    ToggleThemePicker,
    NextView,
    PreviousView,
    SelectView(View),
    /// Move a list cursor, or the theme picker's selection.
    Up,
    Down,
    PageUp,
    PageDown,
    /// `Ctrl-d` / `Ctrl-u`.
    HalfPageDown,
    HalfPageUp,
    Home,
    End,
    /// Freeze the display without dropping the connection.
    TogglePause,
    /// Poll immediately.
    Refresh,
    /// Slow the poll cadence down / speed it up.
    SlowerPolling,
    FasterPolling,
    /// Set the cadence outright, from `:set poll <ms>`.
    SetPollInterval(u64),
    /// Toggle Unicode block drawing, for terminals without a good font.
    ToggleUnicode,
    /// Toggle the tab-strip icons.
    ToggleIcons,
    /// Apply a theme by name, from `:theme <id>` or the palette.
    SetTheme(&'static Theme),
    /// Open the `:` command line.
    OpenCommandLine,
    /// Open the fuzzy command palette.
    OpenPalette,
    /// Open the palette pre-filtered to themes / views.
    OpenPaletteFiltered(&'static str),
    /// Text entry inside the command line or palette.
    Input(char),
    Backspace,
    /// `Ctrl-w` — rub out the word behind the cursor.
    DeleteWordBack,
    /// `Ctrl-u` in an input — clear the line.
    ClearLine,
    /// Tab / Shift-Tab completion in the command line.
    CompleteNext,
    CompletePrevious,
    /// Walk the command history.
    HistoryPrevious,
    HistoryNext,
    /// Commit the overlay's selection, or run the typed command.
    Confirm,
    /// Dismiss the overlay, reverting anything it previewed.
    Cancel,
}

/// One entry in a which-key menu: either a binding or a nested group.
pub enum Entry {
    Leaf {
        key: char,
        label: &'static str,
        action: Action,
    },
    Group {
        key: char,
        label: &'static str,
        entries: &'static [Entry],
    },
}

impl Entry {
    pub fn key(&self) -> char {
        match self {
            Entry::Leaf { key, .. } | Entry::Group { key, .. } => *key,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Entry::Leaf { label, .. } | Entry::Group { label, .. } => label,
        }
    }

    pub fn is_group(&self) -> bool {
        matches!(self, Entry::Group { .. })
    }
}

/// `<leader>f` — find things.
const FIND_MAP: &[Entry] = &[
    Entry::Leaf {
        key: 'f',
        label: "Commands",
        action: Action::OpenPalette,
    },
    Entry::Leaf {
        key: 't',
        label: "Themes",
        action: Action::OpenPaletteFiltered("theme"),
    },
    Entry::Leaf {
        key: 'v',
        label: "Views",
        action: Action::OpenPaletteFiltered("view"),
    },
];

/// `<leader>b` — views, which are this app's buffers.
const BUFFER_MAP: &[Entry] = &[
    Entry::Leaf {
        key: 'b',
        label: "Pick a view",
        action: Action::OpenPaletteFiltered("view"),
    },
    Entry::Leaf {
        key: 'n',
        label: "Next view",
        action: Action::NextView,
    },
    Entry::Leaf {
        key: 'p',
        label: "Previous view",
        action: Action::PreviousView,
    },
    Entry::Leaf {
        key: '1',
        label: "Overview",
        action: Action::SelectView(View::Overview),
    },
    Entry::Leaf {
        key: '2',
        label: "Performance",
        action: Action::SelectView(View::Performance),
    },
    Entry::Leaf {
        key: '3',
        label: "Stream",
        action: Action::SelectView(View::Stream),
    },
    Entry::Leaf {
        key: '4',
        label: "Scenes",
        action: Action::SelectView(View::Scenes),
    },
    Entry::Leaf {
        key: '5',
        label: "Audio",
        action: Action::SelectView(View::Audio),
    },
    Entry::Leaf {
        key: '6',
        label: "Log",
        action: Action::SelectView(View::Log),
    },
];

/// `<leader>u` — UI toggles, as in AstroNvim.
const UI_MAP: &[Entry] = &[
    Entry::Leaf {
        key: 'u',
        label: "Unicode drawing",
        action: Action::ToggleUnicode,
    },
    Entry::Leaf {
        key: 'i',
        label: "Tab icons",
        action: Action::ToggleIcons,
    },
    Entry::Leaf {
        key: 'p',
        label: "Pause the display",
        action: Action::TogglePause,
    },
    Entry::Leaf {
        key: 't',
        label: "Theme picker",
        action: Action::ToggleThemePicker,
    },
];

/// `<leader>p` — polling cadence.
const POLL_MAP: &[Entry] = &[
    Entry::Leaf {
        key: 'f',
        label: "Faster",
        action: Action::FasterPolling,
    },
    Entry::Leaf {
        key: 's',
        label: "Slower",
        action: Action::SlowerPolling,
    },
    Entry::Leaf {
        key: 'r',
        label: "Refresh now",
        action: Action::Refresh,
    },
];

/// The root leader menu.
pub const LEADER_MAP: &[Entry] = &[
    Entry::Group {
        key: 'f',
        label: "Find",
        entries: FIND_MAP,
    },
    Entry::Group {
        key: 'b',
        label: "Views",
        entries: BUFFER_MAP,
    },
    Entry::Group {
        key: 'u',
        label: "UI toggles",
        entries: UI_MAP,
    },
    Entry::Group {
        key: 'p',
        label: "Polling",
        entries: POLL_MAP,
    },
    Entry::Leaf {
        key: 't',
        label: "Theme picker",
        action: Action::ToggleThemePicker,
    },
    Entry::Leaf {
        key: 'r',
        label: "Refresh now",
        action: Action::Refresh,
    },
    Entry::Leaf {
        key: ':',
        label: "Command line",
        action: Action::OpenCommandLine,
    },
    Entry::Leaf {
        key: '?',
        label: "Help",
        action: Action::ToggleHelp,
    },
    Entry::Leaf {
        key: 'q',
        label: "Quit",
        action: Action::Quit,
    },
];

/// `g` — vim's go-to prefix.
const GOTO_MAP: &[Entry] = &[Entry::Leaf {
    key: 'g',
    label: "Top of the list",
    action: Action::Home,
}];

/// `]` — forward through things.
const FORWARD_MAP: &[Entry] = &[Entry::Leaf {
    key: 'b',
    label: "Next view",
    action: Action::NextView,
}];

/// `[` — backward through things.
const BACKWARD_MAP: &[Entry] = &[Entry::Leaf {
    key: 'b',
    label: "Previous view",
    action: Action::PreviousView,
}];

/// The menu a pending prefix should show, with the title to print above it.
pub fn menu(pending: Pending) -> Option<(&'static str, &'static [Entry])> {
    match pending {
        Pending::None => None,
        Pending::Leader => Some((LEADER_LABEL, LEADER_MAP)),
        Pending::LeaderGroup(key) => LEADER_MAP.iter().find_map(|entry| match entry {
            Entry::Group {
                key: group,
                label,
                entries,
                ..
            } if *group == key => Some((*label, *entries)),
            _ => None,
        }),
        Pending::Prefix('g') => Some(("g", GOTO_MAP)),
        Pending::Prefix(']') => Some(("]", FORWARD_MAP)),
        Pending::Prefix('[') => Some(("[", BACKWARD_MAP)),
        Pending::Prefix(_) => None,
    }
}

/// One documented binding. The help overlay renders this table verbatim.
pub struct Binding {
    /// Key label using arrow glyphs.
    pub keys: &'static str,
    /// The same label without arrows, for ASCII mode. Equal to `keys` where
    /// the label contains no glyphs to substitute.
    pub keys_ascii: &'static str,
    pub description: &'static str,
    pub section: &'static str,
}

impl Binding {
    /// The label to print, given the Unicode setting.
    pub fn label(&self, unicode: bool) -> &'static str {
        if unicode { self.keys } else { self.keys_ascii }
    }
}

/// The user-facing key reference.
pub const BINDINGS: &[Binding] = &[
    Binding {
        section: "Leader",
        keys: "space",
        keys_ascii: "space",
        description: "Open the which-key menu",
    },
    Binding {
        section: "Leader",
        keys: "space f",
        keys_ascii: "space f",
        description: "Find: commands, themes, views",
    },
    Binding {
        section: "Leader",
        keys: "space b",
        keys_ascii: "space b",
        description: "Views — next, previous, pick",
    },
    Binding {
        section: "Leader",
        keys: "space u",
        keys_ascii: "space u",
        description: "UI toggles",
    },
    Binding {
        section: "Leader",
        keys: "space p",
        keys_ascii: "space p",
        description: "Polling cadence",
    },
    Binding {
        section: "Leader",
        keys: "space q",
        keys_ascii: "space q",
        description: "Quit",
    },
    Binding {
        section: "Command",
        keys: ":",
        keys_ascii: ":",
        description: "Command line",
    },
    Binding {
        section: "Command",
        keys: "space f f",
        keys_ascii: "space f f",
        description: "Fuzzy command palette",
    },
    Binding {
        section: "Command",
        keys: "Tab",
        keys_ascii: "Tab",
        description: "Complete (in the command line)",
    },
    Binding {
        section: "Navigation",
        keys: "Tab / →",
        keys_ascii: "Tab / Right",
        description: "Next view",
    },
    Binding {
        section: "Navigation",
        keys: "Shift-Tab / ←",
        keys_ascii: "Shift-Tab / Left",
        description: "Previous view",
    },
    Binding {
        section: "Navigation",
        keys: "L / H",
        keys_ascii: "L / H",
        description: "Next / previous view",
    },
    Binding {
        section: "Navigation",
        keys: "] b / [ b",
        keys_ascii: "] b / [ b",
        description: "Next / previous view",
    },
    Binding {
        section: "Navigation",
        keys: "1 … 6",
        keys_ascii: "1 - 6",
        description: "Jump straight to a view",
    },
    Binding {
        section: "Motions",
        keys: "j / k, ↓ / ↑",
        keys_ascii: "j / k, Down / Up",
        description: "Move the list cursor",
    },
    Binding {
        section: "Motions",
        keys: "Ctrl-d / Ctrl-u",
        keys_ascii: "Ctrl-d / Ctrl-u",
        description: "Half a page down / up",
    },
    Binding {
        section: "Motions",
        keys: "Ctrl-f / Ctrl-b",
        keys_ascii: "Ctrl-f / Ctrl-b",
        description: "A page down / up",
    },
    Binding {
        section: "Motions",
        keys: "g g / G",
        keys_ascii: "g g / G",
        description: "Jump to the top / bottom",
    },
    Binding {
        section: "Data",
        keys: "r",
        keys_ascii: "r",
        description: "Refresh now",
    },
    Binding {
        section: "Data",
        keys: "p",
        keys_ascii: "p",
        description: "Pause or resume the display",
    },
    Binding {
        section: "Data",
        keys: "+ / -",
        keys_ascii: "+ / -",
        description: "Poll faster / slower",
    },
    Binding {
        section: "Appearance",
        keys: "F2 / t",
        keys_ascii: "F2 / t",
        description: "Theme picker",
    },
    Binding {
        section: "Appearance",
        keys: "u",
        keys_ascii: "u",
        description: "Toggle Unicode drawing",
    },
    Binding {
        section: "General",
        keys: "? / F1",
        keys_ascii: "? / F1",
        description: "This help",
    },
    Binding {
        section: "General",
        keys: "q / Ctrl-C",
        keys_ascii: "q / Ctrl-C",
        description: "Quit",
    },
    Binding {
        section: "Overlays",
        keys: "Enter",
        keys_ascii: "Enter",
        description: "Apply and close",
    },
    Binding {
        section: "Overlays",
        keys: "Esc",
        keys_ascii: "Esc",
        description: "Close, reverting the preview",
    },
];

/// Sections in the order the help overlay should print them.
pub const SECTIONS: &[&str] = &[
    "Leader",
    "Command",
    "Navigation",
    "Motions",
    "Data",
    "Appearance",
    "Overlays",
    "General",
];

/// Map a key press to a resolution, given the layer and any pending prefix.
pub fn resolve(overlay: Overlay, pending: Pending, key: KeyEvent) -> Resolution {
    // Ctrl-C means quit everywhere, including mid-sequence and inside
    // overlays. Nothing in this app is destructive enough to justify
    // intercepting it.
    if key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C'))
    {
        return Resolution::Act(Action::Quit);
    }

    // A sequence in flight owns the keyboard until it resolves or is abandoned.
    if pending != Pending::None {
        return resolve_pending(pending, key);
    }

    match overlay {
        Overlay::None => resolve_dashboard(key),
        Overlay::Help => resolve_help(key),
        Overlay::Themes => resolve_themes(key),
        Overlay::Command | Overlay::Palette => resolve_input(overlay, key),
    }
}

/// Continue a partly-typed sequence.
fn resolve_pending(pending: Pending, key: KeyEvent) -> Resolution {
    // Esc abandons the sequence, as it does in vim.
    if key.code == KeyCode::Esc {
        return Resolution::Ignore;
    }
    let KeyCode::Char(typed) = key.code else {
        return Resolution::Ignore;
    };
    let Some((_, entries)) = menu(pending) else {
        return Resolution::Ignore;
    };

    match entries.iter().find(|entry| entry.key() == typed) {
        Some(Entry::Leaf { action, .. }) => Resolution::Act(*action),
        Some(Entry::Group { key, .. }) => {
            // Only the leader nests; a group inside a group would need a
            // deeper Pending and nothing wants one.
            if pending == Pending::Leader {
                Resolution::Wait(Pending::LeaderGroup(*key))
            } else {
                Resolution::Ignore
            }
        }
        None => Resolution::Ignore,
    }
}

fn resolve_dashboard(key: KeyEvent) -> Resolution {
    // Shift-Tab arrives as BackTab, and some terminals additionally set SHIFT.
    if key.code == KeyCode::BackTab {
        return Resolution::Act(Action::PreviousView);
    }
    if key.code == KeyCode::Tab {
        return Resolution::Act(if key.modifiers.contains(KeyModifiers::SHIFT) {
            Action::PreviousView
        } else {
            Action::NextView
        });
    }

    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return match key.code {
            KeyCode::Char('d') | KeyCode::Char('D') => Resolution::Act(Action::HalfPageDown),
            KeyCode::Char('u') | KeyCode::Char('U') => Resolution::Act(Action::HalfPageUp),
            KeyCode::Char('f') | KeyCode::Char('F') => Resolution::Act(Action::PageDown),
            KeyCode::Char('b') | KeyCode::Char('B') => Resolution::Act(Action::PageUp),
            _ => Resolution::Ignore,
        };
    }

    let action = match key.code {
        // The leader opens which-key rather than acting on its own.
        KeyCode::Char(' ') => return Resolution::Wait(Pending::Leader),
        KeyCode::Char('g') => return Resolution::Wait(Pending::Prefix('g')),
        KeyCode::Char(']') => return Resolution::Wait(Pending::Prefix(']')),
        KeyCode::Char('[') => return Resolution::Wait(Pending::Prefix('[')),
        KeyCode::Char(':') => Action::OpenCommandLine,
        KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => Action::Quit,
        KeyCode::Char('?') | KeyCode::F(1) => Action::ToggleHelp,
        KeyCode::F(2) | KeyCode::Char('t') | KeyCode::Char('T') => Action::ToggleThemePicker,
        // Vim's window-less buffer walk, as AstroNvim binds it.
        KeyCode::Char('L') => Action::NextView,
        KeyCode::Char('H') => Action::PreviousView,
        KeyCode::Right | KeyCode::Char('l') => Action::NextView,
        KeyCode::Left | KeyCode::Char('h') => Action::PreviousView,
        KeyCode::Down | KeyCode::Char('j') => Action::Down,
        KeyCode::Up | KeyCode::Char('k') => Action::Up,
        KeyCode::PageDown => Action::PageDown,
        KeyCode::PageUp => Action::PageUp,
        KeyCode::Home => Action::Home,
        KeyCode::End | KeyCode::Char('G') => Action::End,
        KeyCode::Char('r') | KeyCode::Char('R') => Action::Refresh,
        KeyCode::Char('p') | KeyCode::Char('P') => Action::TogglePause,
        // '=' is the unshifted '+' on most layouts; accept both.
        KeyCode::Char('+') | KeyCode::Char('=') => Action::FasterPolling,
        KeyCode::Char('-') | KeyCode::Char('_') => Action::SlowerPolling,
        KeyCode::Char('u') | KeyCode::Char('U') => Action::ToggleUnicode,
        KeyCode::Char(digit @ '1'..='9') => {
            let index = digit as usize - '1' as usize;
            return match View::from_index(index) {
                Some(view) => Resolution::Act(Action::SelectView(view)),
                None => Resolution::Ignore,
            };
        }
        _ => return Resolution::Ignore,
    };
    Resolution::Act(action)
}

fn resolve_help(key: KeyEvent) -> Resolution {
    match key.code {
        KeyCode::Esc | KeyCode::Enter | KeyCode::Char('?') | KeyCode::F(1) => {
            Resolution::Act(Action::Cancel)
        }
        KeyCode::Char('q') | KeyCode::Char('Q') => Resolution::Act(Action::Quit),
        KeyCode::Down | KeyCode::Char('j') => Resolution::Act(Action::Down),
        KeyCode::Up | KeyCode::Char('k') => Resolution::Act(Action::Up),
        _ => Resolution::Ignore,
    }
}

fn resolve_themes(key: KeyEvent) -> Resolution {
    match key.code {
        KeyCode::Enter => Resolution::Act(Action::Confirm),
        KeyCode::Esc | KeyCode::F(2) => Resolution::Act(Action::Cancel),
        KeyCode::Char('q') | KeyCode::Char('Q') => Resolution::Act(Action::Quit),
        KeyCode::Down | KeyCode::Char('j') => Resolution::Act(Action::Down),
        KeyCode::Up | KeyCode::Char('k') => Resolution::Act(Action::Up),
        KeyCode::PageDown => Resolution::Act(Action::PageDown),
        KeyCode::PageUp => Resolution::Act(Action::PageUp),
        KeyCode::Home | KeyCode::Char('g') => Resolution::Act(Action::Home),
        KeyCode::End | KeyCode::Char('G') => Resolution::Act(Action::End),
        _ => Resolution::Ignore,
    }
}

/// The command line and the palette are both text inputs, so they share a
/// keymap: printable characters go in, and the rest edits or commits.
///
/// `q` must *not* quit here — it is a letter someone is typing.
fn resolve_input(overlay: Overlay, key: KeyEvent) -> Resolution {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return match key.code {
            KeyCode::Char('w') | KeyCode::Char('W') => Resolution::Act(Action::DeleteWordBack),
            KeyCode::Char('u') | KeyCode::Char('U') => Resolution::Act(Action::ClearLine),
            KeyCode::Char('n') | KeyCode::Char('N') => Resolution::Act(Action::Down),
            KeyCode::Char('p') | KeyCode::Char('P') => Resolution::Act(Action::Up),
            _ => Resolution::Ignore,
        };
    }

    match key.code {
        KeyCode::Esc => Resolution::Act(Action::Cancel),
        KeyCode::Enter => Resolution::Act(Action::Confirm),
        KeyCode::Backspace => Resolution::Act(Action::Backspace),
        KeyCode::BackTab => Resolution::Act(Action::CompletePrevious),
        KeyCode::Tab => Resolution::Act(Action::CompleteNext),
        KeyCode::Down => Resolution::Act(Action::Down),
        KeyCode::Up => {
            // In the command line, Up walks history; in the palette it moves
            // the selection.
            Resolution::Act(if overlay == Overlay::Command {
                Action::HistoryPrevious
            } else {
                Action::Up
            })
        }
        KeyCode::Char(c) => Resolution::Act(Action::Input(c)),
        _ => Resolution::Ignore,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    fn dashboard(code: KeyCode) -> Resolution {
        resolve(Overlay::None, Pending::None, key(code))
    }

    /// Type a whole sequence from the dashboard and return the final
    /// resolution, exactly as the app's event loop would thread it.
    fn sequence(codes: &[KeyCode]) -> Resolution {
        let mut pending = Pending::None;
        let mut last = Resolution::Ignore;
        for code in codes {
            last = resolve(Overlay::None, pending, key(*code));
            pending = match last {
                Resolution::Wait(next) => next,
                _ => Pending::None,
            };
        }
        last
    }

    #[test]
    fn ctrl_c_quits_from_every_layer() {
        for overlay in [
            Overlay::None,
            Overlay::Help,
            Overlay::Themes,
            Overlay::Command,
            Overlay::Palette,
        ] {
            assert_eq!(
                resolve(overlay, Pending::None, ctrl(KeyCode::Char('c'))),
                Resolution::Act(Action::Quit)
            );
        }
        // ...including part-way through a leader sequence.
        assert_eq!(
            resolve(Overlay::None, Pending::Leader, ctrl(KeyCode::Char('c'))),
            Resolution::Act(Action::Quit)
        );
    }

    #[test]
    fn space_opens_the_leader_menu_rather_than_pausing() {
        assert_eq!(
            dashboard(KeyCode::Char(' ')),
            Resolution::Wait(Pending::Leader)
        );
        // Pause moved to `p`, which is where it stays.
        assert_eq!(
            dashboard(KeyCode::Char('p')),
            Resolution::Act(Action::TogglePause)
        );
    }

    #[test]
    fn leader_sequences_reach_their_actions() {
        assert_eq!(
            sequence(&[KeyCode::Char(' '), KeyCode::Char('q')]),
            Resolution::Act(Action::Quit)
        );
        assert_eq!(
            sequence(&[KeyCode::Char(' '), KeyCode::Char('t')]),
            Resolution::Act(Action::ToggleThemePicker)
        );
        assert_eq!(
            sequence(&[KeyCode::Char(' '), KeyCode::Char('r')]),
            Resolution::Act(Action::Refresh)
        );
    }

    #[test]
    fn leader_groups_nest_one_level_then_act() {
        // <leader>u opens the UI group, <leader>uu toggles Unicode.
        assert_eq!(
            sequence(&[KeyCode::Char(' '), KeyCode::Char('u')]),
            Resolution::Wait(Pending::LeaderGroup('u'))
        );
        assert_eq!(
            sequence(&[KeyCode::Char(' '), KeyCode::Char('u'), KeyCode::Char('u')]),
            Resolution::Act(Action::ToggleUnicode)
        );
        assert_eq!(
            sequence(&[KeyCode::Char(' '), KeyCode::Char('u'), KeyCode::Char('p')]),
            Resolution::Act(Action::TogglePause)
        );
        assert_eq!(
            sequence(&[KeyCode::Char(' '), KeyCode::Char('b'), KeyCode::Char('4')]),
            Resolution::Act(Action::SelectView(View::Scenes))
        );
        assert_eq!(
            sequence(&[KeyCode::Char(' '), KeyCode::Char('f'), KeyCode::Char('f')]),
            Resolution::Act(Action::OpenPalette)
        );
    }

    #[test]
    fn an_unmapped_key_abandons_the_sequence() {
        assert_eq!(
            sequence(&[KeyCode::Char(' '), KeyCode::Char('z')]),
            Resolution::Ignore
        );
        // Esc abandons it too, rather than quitting the app.
        assert_eq!(
            resolve(Overlay::None, Pending::Leader, key(KeyCode::Esc)),
            Resolution::Ignore
        );
    }

    #[test]
    fn gg_jumps_to_the_top_and_g_alone_waits() {
        assert_eq!(
            dashboard(KeyCode::Char('g')),
            Resolution::Wait(Pending::Prefix('g'))
        );
        assert_eq!(
            sequence(&[KeyCode::Char('g'), KeyCode::Char('g')]),
            Resolution::Act(Action::Home)
        );
        // Shift-G still goes to the bottom in one press, as in vim.
        assert_eq!(dashboard(KeyCode::Char('G')), Resolution::Act(Action::End));
        // Home is untouched, so the pre-vim binding still works.
        assert_eq!(dashboard(KeyCode::Home), Resolution::Act(Action::Home));
    }

    #[test]
    fn bracket_pairs_walk_the_views() {
        assert_eq!(
            sequence(&[KeyCode::Char(']'), KeyCode::Char('b')]),
            Resolution::Act(Action::NextView)
        );
        assert_eq!(
            sequence(&[KeyCode::Char('['), KeyCode::Char('b')]),
            Resolution::Act(Action::PreviousView)
        );
    }

    #[test]
    fn shift_hl_walks_the_views_like_astronvim() {
        assert_eq!(
            dashboard(KeyCode::Char('L')),
            Resolution::Act(Action::NextView)
        );
        assert_eq!(
            dashboard(KeyCode::Char('H')),
            Resolution::Act(Action::PreviousView)
        );
    }

    #[test]
    fn control_motions_page_the_list() {
        assert_eq!(
            resolve(Overlay::None, Pending::None, ctrl(KeyCode::Char('d'))),
            Resolution::Act(Action::HalfPageDown)
        );
        assert_eq!(
            resolve(Overlay::None, Pending::None, ctrl(KeyCode::Char('u'))),
            Resolution::Act(Action::HalfPageUp)
        );
        assert_eq!(
            resolve(Overlay::None, Pending::None, ctrl(KeyCode::Char('f'))),
            Resolution::Act(Action::PageDown)
        );
        assert_eq!(
            resolve(Overlay::None, Pending::None, ctrl(KeyCode::Char('b'))),
            Resolution::Act(Action::PageUp)
        );
    }

    #[test]
    fn colon_opens_the_command_line() {
        assert_eq!(
            dashboard(KeyCode::Char(':')),
            Resolution::Act(Action::OpenCommandLine)
        );
        assert_eq!(
            sequence(&[KeyCode::Char(' '), KeyCode::Char(':')]),
            Resolution::Act(Action::OpenCommandLine)
        );
    }

    #[test]
    fn the_command_line_takes_text_rather_than_dashboard_keys() {
        // `q` here is a letter, not a quit — the single most important thing
        // a text input must get right.
        assert_eq!(
            resolve(Overlay::Command, Pending::None, key(KeyCode::Char('q'))),
            Resolution::Act(Action::Input('q'))
        );
        assert_eq!(
            resolve(Overlay::Command, Pending::None, key(KeyCode::Char(' '))),
            Resolution::Act(Action::Input(' ')),
            "space is a separator here, not the leader"
        );
        assert_eq!(
            resolve(Overlay::Command, Pending::None, key(KeyCode::Tab)),
            Resolution::Act(Action::CompleteNext)
        );
        assert_eq!(
            resolve(Overlay::Command, Pending::None, key(KeyCode::Enter)),
            Resolution::Act(Action::Confirm)
        );
        assert_eq!(
            resolve(Overlay::Command, Pending::None, key(KeyCode::Esc)),
            Resolution::Act(Action::Cancel)
        );
        assert_eq!(
            resolve(Overlay::Command, Pending::None, ctrl(KeyCode::Char('w'))),
            Resolution::Act(Action::DeleteWordBack)
        );
    }

    #[test]
    fn up_means_history_in_the_command_line_but_selection_in_the_palette() {
        assert_eq!(
            resolve(Overlay::Command, Pending::None, key(KeyCode::Up)),
            Resolution::Act(Action::HistoryPrevious)
        );
        assert_eq!(
            resolve(Overlay::Palette, Pending::None, key(KeyCode::Up)),
            Resolution::Act(Action::Up)
        );
        // Ctrl-n/Ctrl-p move the selection in both, as in Telescope.
        assert_eq!(
            resolve(Overlay::Palette, Pending::None, ctrl(KeyCode::Char('n'))),
            Resolution::Act(Action::Down)
        );
    }

    #[test]
    fn quitting_from_the_dashboard() {
        assert_eq!(dashboard(KeyCode::Char('q')), Resolution::Act(Action::Quit));
        assert_eq!(dashboard(KeyCode::Char('Q')), Resolution::Act(Action::Quit));
        assert_eq!(dashboard(KeyCode::Esc), Resolution::Act(Action::Quit));
    }

    #[test]
    fn tab_and_back_tab_walk_the_views() {
        assert_eq!(dashboard(KeyCode::Tab), Resolution::Act(Action::NextView));
        assert_eq!(
            dashboard(KeyCode::BackTab),
            Resolution::Act(Action::PreviousView)
        );
        // Some terminals send Tab with SHIFT rather than BackTab.
        assert_eq!(
            resolve(
                Overlay::None,
                Pending::None,
                KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT)
            ),
            Resolution::Act(Action::PreviousView)
        );
        // ...and others send BackTab *with* SHIFT set.
        assert_eq!(
            resolve(
                Overlay::None,
                Pending::None,
                KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT)
            ),
            Resolution::Act(Action::PreviousView)
        );
    }

    #[test]
    fn hjkl_and_arrows_are_equivalent() {
        assert_eq!(dashboard(KeyCode::Char('j')), dashboard(KeyCode::Down));
        assert_eq!(dashboard(KeyCode::Char('k')), dashboard(KeyCode::Up));
        assert_eq!(dashboard(KeyCode::Char('l')), dashboard(KeyCode::Right));
        assert_eq!(dashboard(KeyCode::Char('h')), dashboard(KeyCode::Left));
    }

    #[test]
    fn digits_select_views_and_stop_at_the_last_one() {
        assert_eq!(
            dashboard(KeyCode::Char('1')),
            Resolution::Act(Action::SelectView(View::Overview))
        );
        assert_eq!(
            dashboard(KeyCode::Char('6')),
            Resolution::Act(Action::SelectView(View::Log))
        );
        // There is no seventh view, so the key does nothing rather than
        // silently landing on the wrong tab.
        assert_eq!(dashboard(KeyCode::Char('7')), Resolution::Ignore);
        assert_eq!(dashboard(KeyCode::Char('9')), Resolution::Ignore);
    }

    #[test]
    fn every_view_is_reachable_by_a_digit() {
        for view in View::ALL {
            let digit = char::from_digit(view.index() as u32 + 1, 10).unwrap();
            assert_eq!(
                dashboard(KeyCode::Char(digit)),
                Resolution::Act(Action::SelectView(view))
            );
        }
    }

    #[test]
    fn the_pre_vim_keys_all_still_work() {
        // The promise made when the vim layer landed: nothing anyone already
        // learned stopped working.
        for (code, action) in [
            (KeyCode::Char('r'), Action::Refresh),
            (KeyCode::Char('p'), Action::TogglePause),
            (KeyCode::Char('+'), Action::FasterPolling),
            (KeyCode::Char('='), Action::FasterPolling),
            (KeyCode::Char('-'), Action::SlowerPolling),
            (KeyCode::Char('u'), Action::ToggleUnicode),
            (KeyCode::Char('t'), Action::ToggleThemePicker),
            (KeyCode::F(2), Action::ToggleThemePicker),
            (KeyCode::Char('?'), Action::ToggleHelp),
            (KeyCode::F(1), Action::ToggleHelp),
            (KeyCode::PageDown, Action::PageDown),
            (KeyCode::PageUp, Action::PageUp),
            (KeyCode::Home, Action::Home),
            (KeyCode::End, Action::End),
        ] {
            assert_eq!(
                dashboard(code),
                Resolution::Act(action),
                "{code:?} regressed"
            );
        }
    }

    #[test]
    fn unbound_keys_resolve_to_nothing() {
        assert_eq!(dashboard(KeyCode::Char('z')), Resolution::Ignore);
        assert_eq!(dashboard(KeyCode::F(12)), Resolution::Ignore);
        assert_eq!(dashboard(KeyCode::Insert), Resolution::Ignore);
    }

    #[test]
    fn help_closes_on_escape_enter_or_its_own_key() {
        for code in [
            KeyCode::Esc,
            KeyCode::Enter,
            KeyCode::Char('?'),
            KeyCode::F(1),
        ] {
            assert_eq!(
                resolve(Overlay::Help, Pending::None, key(code)),
                Resolution::Act(Action::Cancel)
            );
        }
    }

    #[test]
    fn help_does_not_swallow_view_switching_into_a_wrong_action() {
        // Tab has no meaning while help is up; it must not fall through to
        // the dashboard's NextView.
        assert_eq!(
            resolve(Overlay::Help, Pending::None, key(KeyCode::Tab)),
            Resolution::Ignore
        );
        assert_eq!(
            resolve(Overlay::Help, Pending::None, key(KeyCode::Char('1'))),
            Resolution::Ignore
        );
    }

    #[test]
    fn theme_picker_separates_confirm_from_cancel() {
        assert_eq!(
            resolve(Overlay::Themes, Pending::None, key(KeyCode::Enter)),
            Resolution::Act(Action::Confirm)
        );
        assert_eq!(
            resolve(Overlay::Themes, Pending::None, key(KeyCode::Esc)),
            Resolution::Act(Action::Cancel)
        );
        assert_eq!(
            resolve(Overlay::Themes, Pending::None, key(KeyCode::F(2))),
            Resolution::Act(Action::Cancel)
        );
    }

    #[test]
    fn theme_picker_navigates_with_the_same_keys_as_lists() {
        assert_eq!(
            resolve(Overlay::Themes, Pending::None, key(KeyCode::Char('j'))),
            Resolution::Act(Action::Down)
        );
        assert_eq!(
            resolve(Overlay::Themes, Pending::None, key(KeyCode::Up)),
            Resolution::Act(Action::Up)
        );
        assert_eq!(
            resolve(Overlay::Themes, Pending::None, key(KeyCode::Char('g'))),
            Resolution::Act(Action::Home)
        );
        assert_eq!(
            resolve(Overlay::Themes, Pending::None, key(KeyCode::Char('G'))),
            Resolution::Act(Action::End)
        );
    }

    #[test]
    fn quitting_works_from_inside_the_non_text_overlays() {
        assert_eq!(
            resolve(Overlay::Themes, Pending::None, key(KeyCode::Char('q'))),
            Resolution::Act(Action::Quit)
        );
        assert_eq!(
            resolve(Overlay::Help, Pending::None, key(KeyCode::Char('q'))),
            Resolution::Act(Action::Quit)
        );
    }

    #[test]
    fn every_documented_binding_belongs_to_a_known_section() {
        for binding in BINDINGS {
            assert!(
                SECTIONS.contains(&binding.section),
                "{} is in unknown section {}",
                binding.keys,
                binding.section
            );
            assert!(!binding.keys.is_empty());
            assert!(!binding.keys_ascii.is_empty());
            assert!(!binding.description.is_empty());
        }
    }

    #[test]
    fn ascii_key_labels_contain_no_arrow_glyphs() {
        for binding in BINDINGS {
            assert_eq!(binding.label(true), binding.keys);
            let ascii = binding.label(false);
            assert!(ascii.is_ascii(), "'{ascii}' is not usable in ASCII mode");
        }
    }

    #[test]
    fn every_section_has_at_least_one_binding() {
        for section in SECTIONS {
            assert!(
                BINDINGS.iter().any(|binding| binding.section == *section),
                "{section} has no bindings"
            );
        }
    }

    #[test]
    fn every_leader_entry_is_reachable_and_uniquely_keyed() {
        fn check(entries: &'static [Entry], path: &str) {
            let mut seen = Vec::new();
            for entry in entries {
                assert!(
                    !seen.contains(&entry.key()),
                    "{path}{} is bound twice",
                    entry.key()
                );
                seen.push(entry.key());
                assert!(
                    !entry.label().is_empty(),
                    "{path}{} has no label",
                    entry.key()
                );
                if let Entry::Group { key, entries, .. } = entry {
                    check(entries, &format!("{path}{key} "));
                }
            }
        }
        check(LEADER_MAP, "");
    }

    #[test]
    fn every_leader_group_has_a_menu_to_show() {
        for entry in LEADER_MAP {
            if let Entry::Group { key, .. } = entry {
                assert!(
                    menu(Pending::LeaderGroup(*key)).is_some(),
                    "<leader>{key} opens a group with no menu"
                );
            }
        }
        assert!(menu(Pending::Leader).is_some());
        assert!(menu(Pending::None).is_none());
        assert!(menu(Pending::Prefix('x')).is_none());
    }

    #[test]
    fn which_key_labels_are_ascii_so_the_popup_survives_ascii_mode() {
        fn check(entries: &'static [Entry]) {
            for entry in entries {
                assert!(entry.label().is_ascii(), "{} is not ASCII", entry.label());
                assert!(entry.key().is_ascii());
                if let Entry::Group { entries, .. } = entry {
                    check(entries);
                }
            }
        }
        check(LEADER_MAP);
        check(GOTO_MAP);
        check(FORWARD_MAP);
        check(BACKWARD_MAP);
    }
}
