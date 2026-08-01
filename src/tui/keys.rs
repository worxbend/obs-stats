//! Key bindings.
//!
//! Resolution is a pure function of `(overlay, key)` → [`Action`], with no
//! access to app state. That makes the whole keymap testable without a
//! terminal, and it is why the help overlay can be generated from the same
//! [`BINDINGS`] table that documents the app — the two cannot drift.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::tui::view::View;

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
}

/// Something the app should do in response to a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    Home,
    End,
    /// Freeze the display without dropping the connection.
    TogglePause,
    /// Poll immediately.
    Refresh,
    /// Slow the poll cadence down / speed it up.
    SlowerPolling,
    FasterPolling,
    /// Toggle Unicode block drawing, for terminals without a good font.
    ToggleUnicode,
    /// Commit the overlay's selection.
    Confirm,
    /// Dismiss the overlay, reverting anything it previewed.
    Cancel,
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
        keys: "1 … 6",
        keys_ascii: "1 - 6",
        description: "Jump straight to a view",
    },
    Binding {
        section: "Navigation",
        keys: "j / k, ↓ / ↑",
        keys_ascii: "j / k, Down / Up",
        description: "Move the list cursor",
    },
    Binding {
        section: "Navigation",
        keys: "PgDn / PgUp",
        keys_ascii: "PgDn / PgUp",
        description: "Move the cursor a page",
    },
    Binding {
        section: "Navigation",
        keys: "g / G",
        keys_ascii: "g / G",
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
        keys: "space / p",
        keys_ascii: "space / p",
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
        keys: "q / Esc / Ctrl-C",
        keys_ascii: "q / Esc / Ctrl-C",
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
pub const SECTIONS: &[&str] = &["Navigation", "Data", "Appearance", "Overlays", "General"];

/// Map a key press to an action, given the overlay that owns the keyboard.
///
/// Returns `None` for keys with no meaning in the current layer.
pub fn resolve(overlay: Overlay, key: KeyEvent) -> Option<Action> {
    // Ctrl-C means quit everywhere, including inside overlays. Nothing in this
    // app is destructive enough to justify intercepting it.
    if key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C'))
    {
        return Some(Action::Quit);
    }

    match overlay {
        Overlay::None => resolve_dashboard(key),
        Overlay::Help => resolve_help(key),
        Overlay::Themes => resolve_themes(key),
    }
}

fn resolve_dashboard(key: KeyEvent) -> Option<Action> {
    // Shift-Tab arrives as BackTab, and some terminals additionally set SHIFT.
    if key.code == KeyCode::BackTab {
        return Some(Action::PreviousView);
    }
    if key.code == KeyCode::Tab {
        return Some(if key.modifiers.contains(KeyModifiers::SHIFT) {
            Action::PreviousView
        } else {
            Action::NextView
        });
    }

    let action = match key.code {
        KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => Action::Quit,
        KeyCode::Char('?') | KeyCode::F(1) => Action::ToggleHelp,
        KeyCode::F(2) | KeyCode::Char('t') | KeyCode::Char('T') => Action::ToggleThemePicker,
        KeyCode::Right | KeyCode::Char('l') => Action::NextView,
        KeyCode::Left | KeyCode::Char('h') => Action::PreviousView,
        KeyCode::Down | KeyCode::Char('j') => Action::Down,
        KeyCode::Up | KeyCode::Char('k') => Action::Up,
        KeyCode::PageDown => Action::PageDown,
        KeyCode::PageUp => Action::PageUp,
        KeyCode::Home | KeyCode::Char('g') => Action::Home,
        KeyCode::End | KeyCode::Char('G') => Action::End,
        KeyCode::Char('r') | KeyCode::Char('R') => Action::Refresh,
        KeyCode::Char(' ') | KeyCode::Char('p') | KeyCode::Char('P') => Action::TogglePause,
        // '=' is the unshifted '+' on most layouts; accept both.
        KeyCode::Char('+') | KeyCode::Char('=') => Action::FasterPolling,
        KeyCode::Char('-') | KeyCode::Char('_') => Action::SlowerPolling,
        KeyCode::Char('u') | KeyCode::Char('U') => Action::ToggleUnicode,
        KeyCode::Char(digit @ '1'..='9') => {
            let index = digit as usize - '1' as usize;
            return View::from_index(index).map(Action::SelectView);
        }
        _ => return None,
    };
    Some(action)
}

fn resolve_help(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Esc | KeyCode::Enter | KeyCode::Char('?') | KeyCode::F(1) => Some(Action::Cancel),
        KeyCode::Char('q') | KeyCode::Char('Q') => Some(Action::Quit),
        KeyCode::Down | KeyCode::Char('j') => Some(Action::Down),
        KeyCode::Up | KeyCode::Char('k') => Some(Action::Up),
        _ => None,
    }
}

fn resolve_themes(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Enter => Some(Action::Confirm),
        KeyCode::Esc | KeyCode::F(2) => Some(Action::Cancel),
        KeyCode::Char('q') | KeyCode::Char('Q') => Some(Action::Quit),
        KeyCode::Down | KeyCode::Char('j') => Some(Action::Down),
        KeyCode::Up | KeyCode::Char('k') => Some(Action::Up),
        KeyCode::PageDown => Some(Action::PageDown),
        KeyCode::PageUp => Some(Action::PageUp),
        KeyCode::Home | KeyCode::Char('g') => Some(Action::Home),
        KeyCode::End | KeyCode::Char('G') => Some(Action::End),
        _ => None,
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

    fn dashboard(code: KeyCode) -> Option<Action> {
        resolve(Overlay::None, key(code))
    }

    #[test]
    fn ctrl_c_quits_from_every_layer() {
        for overlay in [Overlay::None, Overlay::Help, Overlay::Themes] {
            assert_eq!(
                resolve(overlay, ctrl(KeyCode::Char('c'))),
                Some(Action::Quit)
            );
        }
        assert_eq!(
            resolve(Overlay::None, ctrl(KeyCode::Char('C'))),
            Some(Action::Quit)
        );
    }

    #[test]
    fn quitting_from_the_dashboard() {
        assert_eq!(dashboard(KeyCode::Char('q')), Some(Action::Quit));
        assert_eq!(dashboard(KeyCode::Char('Q')), Some(Action::Quit));
        assert_eq!(dashboard(KeyCode::Esc), Some(Action::Quit));
    }

    #[test]
    fn tab_and_back_tab_walk_the_views() {
        assert_eq!(dashboard(KeyCode::Tab), Some(Action::NextView));
        assert_eq!(dashboard(KeyCode::BackTab), Some(Action::PreviousView));
        // Some terminals send Tab with SHIFT rather than BackTab.
        assert_eq!(
            resolve(
                Overlay::None,
                KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT)
            ),
            Some(Action::PreviousView)
        );
        // ...and others send BackTab *with* SHIFT set.
        assert_eq!(
            resolve(
                Overlay::None,
                KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT)
            ),
            Some(Action::PreviousView)
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
            Some(Action::SelectView(View::Overview))
        );
        assert_eq!(
            dashboard(KeyCode::Char('6')),
            Some(Action::SelectView(View::Log))
        );
        // There is no seventh view, so the key does nothing rather than
        // silently landing on the wrong tab.
        assert_eq!(dashboard(KeyCode::Char('7')), None);
        assert_eq!(dashboard(KeyCode::Char('9')), None);
    }

    #[test]
    fn every_view_is_reachable_by_a_digit() {
        for view in View::ALL {
            let digit = char::from_digit(view.index() as u32 + 1, 10).unwrap();
            assert_eq!(
                dashboard(KeyCode::Char(digit)),
                Some(Action::SelectView(view))
            );
        }
    }

    #[test]
    fn data_controls_are_bound() {
        assert_eq!(dashboard(KeyCode::Char('r')), Some(Action::Refresh));
        assert_eq!(dashboard(KeyCode::Char(' ')), Some(Action::TogglePause));
        assert_eq!(dashboard(KeyCode::Char('p')), Some(Action::TogglePause));
        assert_eq!(dashboard(KeyCode::Char('+')), Some(Action::FasterPolling));
        assert_eq!(dashboard(KeyCode::Char('=')), Some(Action::FasterPolling));
        assert_eq!(dashboard(KeyCode::Char('-')), Some(Action::SlowerPolling));
        assert_eq!(dashboard(KeyCode::Char('u')), Some(Action::ToggleUnicode));
    }

    #[test]
    fn help_and_theme_picker_are_bound_to_both_their_keys() {
        assert_eq!(dashboard(KeyCode::Char('?')), Some(Action::ToggleHelp));
        assert_eq!(dashboard(KeyCode::F(1)), Some(Action::ToggleHelp));
        assert_eq!(dashboard(KeyCode::F(2)), Some(Action::ToggleThemePicker));
        assert_eq!(
            dashboard(KeyCode::Char('t')),
            Some(Action::ToggleThemePicker)
        );
    }

    #[test]
    fn unbound_keys_resolve_to_nothing() {
        assert_eq!(dashboard(KeyCode::Char('z')), None);
        assert_eq!(dashboard(KeyCode::F(12)), None);
        assert_eq!(dashboard(KeyCode::Insert), None);
    }

    #[test]
    fn help_closes_on_escape_enter_or_its_own_key() {
        for code in [
            KeyCode::Esc,
            KeyCode::Enter,
            KeyCode::Char('?'),
            KeyCode::F(1),
        ] {
            assert_eq!(resolve(Overlay::Help, key(code)), Some(Action::Cancel));
        }
    }

    #[test]
    fn help_does_not_swallow_view_switching_into_a_wrong_action() {
        // Tab has no meaning while help is up; it must not fall through to
        // the dashboard's NextView.
        assert_eq!(resolve(Overlay::Help, key(KeyCode::Tab)), None);
        assert_eq!(resolve(Overlay::Help, key(KeyCode::Char('1'))), None);
    }

    #[test]
    fn theme_picker_separates_confirm_from_cancel() {
        assert_eq!(
            resolve(Overlay::Themes, key(KeyCode::Enter)),
            Some(Action::Confirm)
        );
        assert_eq!(
            resolve(Overlay::Themes, key(KeyCode::Esc)),
            Some(Action::Cancel)
        );
        assert_eq!(
            resolve(Overlay::Themes, key(KeyCode::F(2))),
            Some(Action::Cancel)
        );
    }

    #[test]
    fn theme_picker_navigates_with_the_same_keys_as_lists() {
        assert_eq!(
            resolve(Overlay::Themes, key(KeyCode::Char('j'))),
            Some(Action::Down)
        );
        assert_eq!(resolve(Overlay::Themes, key(KeyCode::Up)), Some(Action::Up));
        assert_eq!(
            resolve(Overlay::Themes, key(KeyCode::Char('g'))),
            Some(Action::Home)
        );
        assert_eq!(
            resolve(Overlay::Themes, key(KeyCode::Char('G'))),
            Some(Action::End)
        );
        assert_eq!(
            resolve(Overlay::Themes, key(KeyCode::PageDown)),
            Some(Action::PageDown)
        );
    }

    #[test]
    fn quitting_works_from_inside_overlays() {
        assert_eq!(
            resolve(Overlay::Themes, key(KeyCode::Char('q'))),
            Some(Action::Quit)
        );
        assert_eq!(
            resolve(Overlay::Help, key(KeyCode::Char('q'))),
            Some(Action::Quit)
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
}
