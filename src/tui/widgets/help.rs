//! The help overlay.
//!
//! Its contents are generated from [`crate::tui::keys::BINDINGS`] and
//! [`View::ALL`], so the documentation and the keymap cannot drift apart.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::tui::app::App;
use crate::tui::keys::{BINDINGS, SECTIONS};
use crate::tui::theme::Theme;
use crate::tui::view::View;
use crate::tui::widgets::chrome;

/// Width of the key column, wide enough for the longest binding.
const KEY_WIDTH: usize = 16;

/// Draw the overlay, returning its panel so a click outside can dismiss it.
pub fn render(frame: &mut Frame, area: Rect, app: &App) -> Rect {
    let lines = body(app.theme, app.show_icons, app.unicode);
    // +2 for the border, +1 so a scrollbar-less panel never clips its last row.
    let height = (lines.len() as u16 + 3).min(area.height);
    let region = chrome::centered(area, 66.min(area.width), height);

    frame.render_widget(Clear, region);
    let block = chrome::panel("Help", true, app.theme, app.unicode)
        .style(Style::default().bg(app.theme.bg));
    frame.render_widget(Paragraph::new(lines).block(block), region);
    region
}

/// The overlay's text, as a flat list of lines.
///
/// Split out so the content can be asserted on without a terminal.
pub fn body(theme: &Theme, show_icons: bool, unicode: bool) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    lines.push(heading("Views", theme));
    for view in View::ALL {
        let key = format!("{}", view.index() + 1);
        let title = if show_icons {
            format!("{} {}", view.icon(), view.title())
        } else {
            view.title().to_string()
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {key:<width$}", width = KEY_WIDTH - 2),
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("{title:<16}"), Style::default().fg(theme.fg)),
            Span::styled(
                view.description().to_string(),
                Style::default().fg(theme.muted),
            ),
        ]));
    }

    for section in SECTIONS {
        lines.push(Line::from(""));
        lines.push(heading(section, theme));
        for binding in BINDINGS.iter().filter(|b| b.section == *section) {
            lines.push(Line::from(vec![
                Span::styled(
                    format!(
                        "  {:<width$}",
                        binding.label(unicode),
                        width = KEY_WIDTH - 2
                    ),
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    binding.description.to_string(),
                    Style::default().fg(theme.fg),
                ),
            ]));
        }
    }

    lines
}

fn heading(text: &str, theme: &Theme) -> Line<'static> {
    Line::from(Span::styled(
        text.to_string(),
        Style::default()
            .fg(theme.accent_alt)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::theme;

    fn rendered(show_icons: bool) -> String {
        body(theme::FALLBACK, show_icons, true)
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn every_view_is_documented_with_its_digit() {
        let text = rendered(false);
        for view in View::ALL {
            assert!(
                text.contains(view.title()),
                "{} is undocumented",
                view.title()
            );
            assert!(
                text.contains(view.description()),
                "{} has no description in help",
                view.title()
            );
        }
        assert!(text.contains("  1  "), "digit shortcuts must be shown");
    }

    #[test]
    fn every_binding_and_section_reaches_the_overlay() {
        let text = rendered(false);
        for binding in BINDINGS {
            assert!(
                text.contains(binding.description),
                "binding '{}' is missing from help",
                binding.keys
            );
        }
        for section in SECTIONS {
            assert!(text.contains(section), "section {section} is missing");
        }
    }

    #[test]
    fn icons_appear_only_when_enabled() {
        assert!(rendered(true).contains(View::Audio.icon()));
        assert!(!rendered(false).contains(View::Audio.icon()));
    }

    #[test]
    fn sections_appear_in_their_declared_order() {
        let text = rendered(false);
        let mut cursor = 0;
        for section in SECTIONS {
            let at = text[cursor..]
                .find(section)
                .unwrap_or_else(|| panic!("{section} out of order"));
            cursor += at;
        }
    }

    #[test]
    fn the_overlay_is_not_empty() {
        assert!(body(theme::FALLBACK, true, true).len() > View::ALL.len() + BINDINGS.len());
    }
}
