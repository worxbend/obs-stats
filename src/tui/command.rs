//! The command registry, and the parsing behind `:` and the fuzzy palette.
//!
//! One table, two front ends. [`COMMANDS`] is the single source of truth for
//! what the app can be told to do by name; the `:` line parses against it and
//! the palette flattens it into runnable items. Because both read the same
//! table they cannot drift, which is the same reason the help overlay is
//! generated from [`crate::tui::keys::BINDINGS`].
//!
//! Everything here is pure: parsing and completion take strings and return
//! values, with no access to app state and no I/O.
//!
//! Argument-free commands carry their [`Action`] in the table itself, so a
//! [`CommandSpec`] that compiles is one that runs.

use crate::config::model::{MAX_POLL_INTERVAL_MS, MIN_POLL_INTERVAL_MS};
use crate::tui::keys::Action;
use crate::tui::theme;
use crate::tui::view::View;

/// What a command's single argument names, which is also what completes it.
#[derive(Debug, Clone, Copy)]
pub enum ArgKind {
    /// The command takes no argument, and this is what running it does.
    ///
    /// Carrying the action here rather than looking it up by name means a
    /// `CommandSpec` that compiles is one that runs: there is no second table
    /// to keep in step, and no way to add a command the parser does not know
    /// about.
    None(Action),
    /// A theme id. Optional: bare `:theme` opens the picker.
    Theme,
    /// A view id. Optional: bare `:view` opens the palette.
    View,
    /// A poll cadence in milliseconds, or `faster` / `slower`.
    Poll,
}

/// One named command.
pub struct CommandSpec {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub arg: ArgKind,
    pub summary: &'static str,
}

impl CommandSpec {
    /// Whether `word` names this command.
    fn matches(&self, word: &str) -> bool {
        self.name == word || self.aliases.contains(&word)
    }
}

/// Every command, in the order the palette lists them.
pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        name: "help",
        aliases: &["h"],
        arg: ArgKind::None(Action::ToggleHelp),
        summary: "Show the key reference",
    },
    CommandSpec {
        name: "view",
        aliases: &["buffer", "b"],
        arg: ArgKind::View,
        summary: "Switch to a view by name",
    },
    CommandSpec {
        name: "theme",
        aliases: &["colorscheme", "colo"],
        arg: ArgKind::Theme,
        summary: "Apply a theme by name",
    },
    CommandSpec {
        name: "poll",
        aliases: &["interval"],
        arg: ArgKind::Poll,
        summary: "Set the poll cadence in milliseconds",
    },
    CommandSpec {
        name: "refresh",
        aliases: &["e"],
        arg: ArgKind::None(Action::Refresh),
        summary: "Poll OBS immediately",
    },
    CommandSpec {
        name: "pause",
        aliases: &[],
        arg: ArgKind::None(Action::TogglePause),
        summary: "Freeze or resume the display",
    },
    CommandSpec {
        name: "unicode",
        aliases: &["ascii"],
        arg: ArgKind::None(Action::ToggleUnicode),
        summary: "Toggle Unicode block drawing",
    },
    CommandSpec {
        name: "icons",
        aliases: &[],
        arg: ArgKind::None(Action::ToggleIcons),
        summary: "Toggle the tab-strip icons",
    },
    CommandSpec {
        name: "quit",
        aliases: &["q", "qa", "exit"],
        arg: ArgKind::None(Action::Quit),
        summary: "Leave obs-stats",
    },
];

/// Split an input line into its command word and the rest.
///
/// A leading `set` is skipped, so `:set poll 500` and `:poll 500` are the same
/// command — vim users reach for both.
fn split(input: &str) -> (&str, &str) {
    let trimmed = input.trim().strip_prefix(':').unwrap_or(input.trim());
    let trimmed = trimmed.trim_start();
    let (head, rest) = match trimmed.split_once(char::is_whitespace) {
        Some((head, rest)) => (head, rest.trim_start()),
        None => (trimmed, ""),
    };
    if head == "set" && !rest.is_empty() {
        return split(rest);
    }
    (head, rest)
}

/// Look up the command `word` names.
pub fn lookup(word: &str) -> Option<&'static CommandSpec> {
    let word = word.to_ascii_lowercase();
    COMMANDS.iter().find(|spec| spec.matches(&word))
}

/// Turn a typed line into an action, or explain why it cannot be one.
///
/// The error is user-facing: it goes straight into the status bar, so it names
/// what was wrong rather than how the parser is structured.
pub fn parse(input: &str) -> Result<Action, String> {
    let (word, rest) = split(input);
    if word.is_empty() {
        return Err("no command given".to_string());
    }
    let Some(spec) = lookup(word) else {
        return Err(format!("not a command: {word}"));
    };

    match spec.arg {
        ArgKind::None(action) => {
            if !rest.is_empty() {
                return Err(format!(":{} takes no argument", spec.name));
            }
            Ok(action)
        }
        ArgKind::Theme => {
            if rest.is_empty() {
                return Ok(Action::ToggleThemePicker);
            }
            match theme::find(rest) {
                Some(found) => Ok(Action::SetTheme(found)),
                None => Err(format!("no such theme: {rest}")),
            }
        }
        ArgKind::View => {
            if rest.is_empty() {
                return Ok(Action::OpenPaletteFiltered("view"));
            }
            match View::from_id(rest) {
                Some(view) => Ok(Action::SelectView(view)),
                None => Err(format!("no such view: {rest}")),
            }
        }
        ArgKind::Poll => match rest {
            "" => Err("poll needs a value in milliseconds".to_string()),
            "faster" | "-" => Ok(Action::FasterPolling),
            "slower" | "+" => Ok(Action::SlowerPolling),
            value => match value.parse::<u64>() {
                Ok(ms) if (MIN_POLL_INTERVAL_MS..=MAX_POLL_INTERVAL_MS).contains(&ms) => {
                    Ok(Action::SetPollInterval(ms))
                }
                Ok(ms) => Err(format!(
                    "{ms}ms is outside {MIN_POLL_INTERVAL_MS}–{MAX_POLL_INTERVAL_MS}ms"
                )),
                Err(_) => Err(format!("not a number: {value}")),
            },
        },
    }
}

/// Completions for a partly-typed line, as whole replacement lines.
///
/// Completing the command word and completing its argument are the same
/// operation from the caller's side, which is what lets Tab work anywhere on
/// the line without the command line needing to know about arguments.
pub fn complete(input: &str) -> Vec<String> {
    let trimmed = input.trim_start();
    // No space typed yet: the user is still naming the command.
    if !trimmed.contains(char::is_whitespace) {
        let prefix = trimmed.to_ascii_lowercase();
        return COMMANDS
            .iter()
            .filter(|spec| spec.name.starts_with(&prefix))
            .map(|spec| {
                if matches!(spec.arg, ArgKind::None(_)) {
                    spec.name.to_string()
                } else {
                    format!("{} ", spec.name)
                }
            })
            .collect();
    }

    let (word, rest) = split(trimmed);
    let Some(spec) = lookup(word) else {
        return Vec::new();
    };
    let prefix = rest.to_ascii_lowercase();

    let values: Vec<String> = match spec.arg {
        ArgKind::None(_) => Vec::new(),
        ArgKind::Theme => theme::THEMES
            .iter()
            .map(|entry| entry.id.to_string())
            .collect(),
        ArgKind::View => View::ALL.iter().map(|view| view.id().to_string()).collect(),
        ArgKind::Poll => ["250", "500", "1000", "2000", "faster", "slower"]
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
    };

    values
        .into_iter()
        .filter(|value| value.starts_with(&prefix))
        .map(|value| format!("{} {value}", spec.name))
        .collect()
}

/// One runnable row in the fuzzy palette.
#[derive(Debug, Clone, PartialEq)]
pub struct PaletteItem {
    /// What the user reads and types against.
    pub label: String,
    /// Dimmed context on the right.
    pub detail: String,
    /// Which filter this belongs to: `command`, `theme` or `view`.
    pub kind: &'static str,
    pub action: Action,
}

/// Every palette row: the argument-free commands, plus one row per theme and
/// per view so a theme is reachable in one pick rather than two.
pub fn palette_items() -> Vec<PaletteItem> {
    let mut items = Vec::new();

    for spec in COMMANDS {
        // Commands needing an argument are represented by their concrete
        // values below; offering a bare `:theme` here as well would list the
        // same thing twice.
        let ArgKind::None(action) = spec.arg else {
            continue;
        };
        items.push(PaletteItem {
            label: format!(":{}", spec.name),
            detail: spec.summary.to_string(),
            kind: "command",
            action,
        });
    }

    for view in View::ALL {
        items.push(PaletteItem {
            label: format!("view {}", view.id()),
            detail: view.description().to_string(),
            kind: "view",
            action: Action::SelectView(view),
        });
    }

    for entry in theme::THEMES {
        items.push(PaletteItem {
            label: format!("theme {}", entry.id),
            detail: entry.label.to_string(),
            kind: "theme",
            action: Action::SetTheme(entry),
        });
    }

    items
}

/// Score `candidate` against a lowercase `query` as a fuzzy subsequence.
///
/// Returns `None` when the query is not a subsequence at all. Higher is
/// better: consecutive runs and word-start matches score above scattered ones,
/// so `ovw` ranks `view overview` over an incidental o…v…w elsewhere.
pub fn score(candidate: &str, query: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let haystack: Vec<char> = candidate.to_ascii_lowercase().chars().collect();
    let needle: Vec<char> = query.chars().collect();

    let mut total = 0;
    let mut index = 0;
    let mut previous: Option<usize> = None;

    for wanted in needle {
        let found = haystack[index..].iter().position(|c| *c == wanted)? + index;
        let mut points = 1;
        if previous == Some(found.saturating_sub(1)) {
            points += 4; // adjacent to the last match
        }
        if found == 0 || !haystack[found - 1].is_ascii_alphanumeric() {
            points += 3; // start of a word
        }
        total += points;
        previous = Some(found);
        index = found + 1;
    }

    // Prefer shorter candidates when the matched text is otherwise equal.
    Some(total * 100 - haystack.len() as i32)
}

/// Filter and rank `items` against `query`, best first.
pub fn filter(items: &[PaletteItem], query: &str) -> Vec<PaletteItem> {
    let query = query.trim().to_ascii_lowercase();
    let mut scored: Vec<(i32, usize, &PaletteItem)> = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| score(&item.label, &query).map(|points| (points, index, item)))
        .collect();

    // Sort by score, then by original position so equal matches keep the
    // registry's deliberate ordering rather than shuffling frame to frame.
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored
        .into_iter()
        .map(|(_, _, item)| item.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_name_and_alias_is_unique() {
        let mut seen: Vec<&str> = Vec::new();
        for spec in COMMANDS {
            for word in std::iter::once(&spec.name).chain(spec.aliases) {
                assert!(!seen.contains(word), "{word} is registered twice");
                seen.push(word);
            }
        }
    }

    #[test]
    fn every_command_parses_from_its_name_and_every_alias() {
        for spec in COMMANDS {
            // Commands with a required argument are exercised separately.
            if matches!(spec.arg, ArgKind::Poll) {
                continue;
            }
            for word in std::iter::once(&spec.name).chain(spec.aliases) {
                assert!(parse(word).is_ok(), ":{word} does not parse");
            }
            assert!(!spec.summary.is_empty(), "{} has no summary", spec.name);
        }
    }

    #[test]
    fn a_leading_colon_and_a_leading_set_are_both_optional() {
        assert_eq!(parse(":quit"), Ok(Action::Quit));
        assert_eq!(parse("quit"), Ok(Action::Quit));
        assert_eq!(parse("  :quit  "), Ok(Action::Quit));
        assert_eq!(parse(":set poll 500"), Ok(Action::SetPollInterval(500)));
        assert_eq!(parse("poll 500"), Ok(Action::SetPollInterval(500)));
    }

    #[test]
    fn commands_resolve_to_the_actions_they_name() {
        assert_eq!(parse("view audio"), Ok(Action::SelectView(View::Audio)));
        assert_eq!(parse("b log"), Ok(Action::SelectView(View::Log)));
        assert_eq!(parse("refresh"), Ok(Action::Refresh));
        assert_eq!(parse("pause"), Ok(Action::TogglePause));
        assert_eq!(parse("unicode"), Ok(Action::ToggleUnicode));
        assert_eq!(parse("icons"), Ok(Action::ToggleIcons));
        assert_eq!(parse("help"), Ok(Action::ToggleHelp));
        assert_eq!(
            parse("theme nord"),
            Ok(Action::SetTheme(theme::find("nord").unwrap()))
        );
        assert_eq!(
            parse("colo dracula"),
            Ok(Action::SetTheme(theme::find("dracula").unwrap()))
        );
    }

    #[test]
    fn an_argument_command_with_no_argument_opens_its_picker() {
        assert_eq!(parse("theme"), Ok(Action::ToggleThemePicker));
        assert_eq!(parse("view"), Ok(Action::OpenPaletteFiltered("view")));
    }

    #[test]
    fn poll_accepts_numbers_and_directions_within_bounds() {
        assert_eq!(parse("poll faster"), Ok(Action::FasterPolling));
        assert_eq!(parse("poll slower"), Ok(Action::SlowerPolling));
        assert_eq!(
            parse(&format!("poll {MIN_POLL_INTERVAL_MS}")),
            Ok(Action::SetPollInterval(MIN_POLL_INTERVAL_MS))
        );
        assert!(parse("poll 0").is_err(), "below the floor");
        assert!(parse("poll 99999999").is_err(), "above the ceiling");
        assert!(parse("poll soon").is_err());
        assert!(parse("poll").is_err());
    }

    #[test]
    fn errors_name_what_was_wrong_in_words_a_user_can_act_on() {
        assert_eq!(parse("nope").unwrap_err(), "not a command: nope");
        assert_eq!(parse("theme puce").unwrap_err(), "no such theme: puce");
        assert_eq!(parse("view nowhere").unwrap_err(), "no such view: nowhere");
        assert_eq!(parse("quit now").unwrap_err(), ":quit takes no argument");
        assert_eq!(parse("").unwrap_err(), "no command given");
    }

    #[test]
    fn command_names_complete_from_a_prefix() {
        let completions = complete("th");
        assert_eq!(completions, vec!["theme ".to_string()]);
        // A command taking an argument completes with a trailing space, so the
        // next Tab starts on the argument rather than re-completing the name.
        assert!(complete("p").contains(&"pause".to_string()));
        assert!(complete("p").contains(&"poll ".to_string()));
        assert!(complete("zzz").is_empty());
    }

    #[test]
    fn arguments_complete_against_the_right_value_set() {
        let themes = complete("theme no");
        assert!(themes.contains(&"theme nord".to_string()));
        assert!(!themes.iter().any(|value| value.contains("dracula")));

        let views = complete("view a");
        assert_eq!(views, vec!["view audio".to_string()]);

        // Every theme is offered when nothing narrows it.
        assert_eq!(complete("theme ").len(), theme::THEMES.len());
        assert!(complete("poll 5").contains(&"poll 500".to_string()));
    }

    #[test]
    fn completions_round_trip_back_through_the_parser() {
        // Anything Tab can produce must be something Enter can run, or
        // completion would hand the user a line that errors. The exception is
        // a completion ending in a space, which is a command still waiting for
        // its argument rather than a finished line.
        for seed in ["", "t", "theme ", "view ", "poll ", "q"] {
            for completion in complete(seed) {
                if completion.ends_with(' ') {
                    assert!(
                        lookup(completion.trim())
                            .is_some_and(|spec| !matches!(spec.arg, ArgKind::None(_))),
                        "{completion:?} trails a space but takes no argument"
                    );
                    continue;
                }
                assert!(
                    parse(&completion).is_ok(),
                    "completing {seed:?} produced {completion:?}, which does not parse"
                );
            }
        }
    }

    #[test]
    fn the_palette_lists_commands_views_and_themes() {
        let items = palette_items();
        assert!(items.iter().any(|item| item.label == ":quit"));
        assert!(items.iter().any(|item| item.label == "view audio"));
        assert!(items.iter().any(|item| item.label == "theme nord"));
        // Every theme and view is reachable.
        assert_eq!(
            items.iter().filter(|item| item.kind == "theme").count(),
            theme::THEMES.len()
        );
        assert_eq!(
            items.iter().filter(|item| item.kind == "view").count(),
            View::ALL.len()
        );
        // No row is a dead end.
        assert!(items.iter().all(|item| item.action != Action::Cancel));
        assert!(items.iter().all(|item| !item.detail.is_empty()));
    }

    #[test]
    fn fuzzy_matching_accepts_subsequences_and_rejects_the_rest() {
        assert!(score("theme nord", "nord").is_some());
        assert!(
            score("theme nord", "tnd").is_some(),
            "scattered subsequence"
        );
        assert!(score("theme nord", "zz").is_none());
        assert_eq!(score("anything", ""), Some(0), "an empty query matches all");
    }

    #[test]
    fn a_contiguous_match_outranks_a_scattered_one() {
        let contiguous = score("theme nord", "nord").unwrap();
        let scattered = score("theme nord", "tnd").unwrap();
        assert!(
            contiguous > scattered,
            "{contiguous} should beat {scattered}"
        );
    }

    #[test]
    fn filtering_ranks_the_obvious_answer_first() {
        let items = palette_items();
        assert_eq!(filter(&items, "nord")[0].label, "theme nord");
        assert_eq!(filter(&items, "audio")[0].label, "view audio");
        assert_eq!(filter(&items, "quit")[0].label, ":quit");
        // An empty query keeps the registry order rather than shuffling.
        assert_eq!(filter(&items, "").len(), items.len());
        assert_eq!(filter(&items, "")[0].label, items[0].label);
        assert!(filter(&items, "zzzz").is_empty());
    }

    #[test]
    fn filtering_is_stable_for_equally_scored_rows() {
        let items = palette_items();
        let first = filter(&items, "e");
        let second = filter(&items, "e");
        assert_eq!(
            first.iter().map(|i| &i.label).collect::<Vec<_>>(),
            second.iter().map(|i| &i.label).collect::<Vec<_>>()
        );
    }
}
