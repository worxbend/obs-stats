//! The `:` command line, drawn over the status bar.
//!
//! It takes the status row rather than floating over the dashboard, which is
//! where vim puts it and where there is guaranteed to be space even in a
//! cramped terminal. The completion list, when there is one, stacks above.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::tui::app::App;
use crate::tui::command;
use crate::tui::theme::Theme;
use crate::tui::widgets::chrome;

/// Completions listed above the line before it scrolls.
const MAX_COMPLETIONS: u16 = 6;

/// Draw the command line into `status`, with any completions above it.
pub fn render(frame: &mut Frame, area: Rect, status: Rect, app: &App) {
    let completions = command::complete(&app.command.input);
    render_completions(frame, area, status, &completions, app);

    // `Clear` resets the cells it covers, which on a themed terminal means a
    // strip of the user's own wallpaper across the bottom row. Repainting the
    // background is what puts it back.
    frame.render_widget(Clear, status);
    frame.render_widget(
        Paragraph::new(line(app)).style(Style::default().bg(app.theme.bg)),
        status,
    );

    // A real cursor, so the terminal's own blink marks the insertion point
    // rather than the app drawing a fake one that never blinks.
    let column = status.x + 1 + app.command.input.chars().count() as u16;
    frame.set_cursor_position((column.min(status.right().saturating_sub(1)), status.y));
}

/// `:theme nord` with the prompt dimmed.
fn line(app: &App) -> Line<'static> {
    let theme = app.theme;
    Line::from(vec![
        Span::styled(
            ":",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(app.command.input.clone(), Style::default().fg(theme.fg)),
    ])
}

/// The completion list, stacked directly above the command line.
fn render_completions(
    frame: &mut Frame,
    area: Rect,
    status: Rect,
    completions: &[String],
    app: &App,
) {
    // One completion that is already exactly what is typed tells the user
    // nothing, so it is not worth a panel.
    if completions.is_empty() || (completions.len() == 1 && completions[0] == app.command.input) {
        return;
    }

    let rows = (completions.len() as u16).min(MAX_COMPLETIONS);
    let height = rows + 2; // borders
    if status.y < height || area.width < 10 {
        return;
    }

    // The floor keeps the panel from looking cramped, but never widens it past
    // the terminal — `clamp` would panic when the terminal is the narrower of
    // the two.
    let width = completions
        .iter()
        .map(|entry| entry.chars().count() as u16 + 4)
        .max()
        .unwrap_or(20)
        .max(20)
        .min(area.width);

    let region = Rect {
        x: area.x,
        y: status.y - height,
        width,
        height,
    };
    let block = chrome::panel_with_note(
        "Completions",
        format!("{}", completions.len()),
        false,
        app.theme,
        app.unicode,
    );
    let inner = chrome::popup(frame, region, block, app.theme);

    frame.render_widget(
        Paragraph::new(rows_for(completions, rows as usize, app.theme)),
        inner,
    );
}

/// The visible slice of the completion list.
fn rows_for(completions: &[String], rows: usize, theme: &Theme) -> Vec<Line<'static>> {
    completions
        .iter()
        .take(rows)
        .map(|entry| {
            Line::from(Span::styled(
                entry.clone(),
                Style::default().fg(theme.muted),
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::tui::keys::{Action, Overlay};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::time::Instant;

    /// An app with `input` typed into an open command line.
    fn app_with(input: &str) -> App {
        let mut app = App::new(Config::default());
        app.handle(Action::OpenCommandLine, Instant::now());
        for c in input.chars() {
            app.handle(Action::Input(c), Instant::now());
        }
        assert_eq!(app.overlay, Overlay::Command);
        app
    }

    fn render_at(input: &str, width: u16, height: u16) -> String {
        let app = app_with(input);
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                let status = Rect {
                    y: area.bottom() - 1,
                    height: 1,
                    ..area
                };
                render(frame, area, status, &app);
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_typed_line_is_shown_after_a_colon_prompt() {
        let rendered = render_at("theme nord", 80, 24);
        assert!(rendered.contains(":theme nord"), "{rendered}");
    }

    #[test]
    fn completions_are_listed_above_the_line() {
        let rendered = render_at("theme no", 80, 24);
        assert!(rendered.contains("Completions"), "{rendered}");
        assert!(rendered.contains("theme nord"));
    }

    #[test]
    fn an_exact_single_completion_is_not_worth_a_panel() {
        // ":pause" completes only to itself; a one-row popup repeating the
        // line back is noise.
        let rendered = render_at("pause", 80, 24);
        assert!(!rendered.contains("Completions"), "{rendered}");
    }

    #[test]
    fn an_unmatched_line_offers_nothing() {
        let rendered = render_at("wibble", 80, 24);
        assert!(!rendered.contains("Completions"));
        assert!(rendered.contains(":wibble"));
    }

    #[test]
    fn the_completion_list_is_capped_rather_than_filling_the_screen() {
        let app = app_with("theme ");
        let all = command::complete(&app.command.input);
        assert!(all.len() > MAX_COMPLETIONS as usize, "needs a long list");

        let rendered = render_at("theme ", 80, 24);
        let listed = all
            .iter()
            .filter(|entry| rendered.contains(entry.as_str()))
            .count();
        assert!(
            listed <= MAX_COMPLETIONS as usize,
            "{listed} rows drawn, cap is {MAX_COMPLETIONS}"
        );
    }

    #[test]
    fn a_terminal_too_short_for_the_panel_still_draws_the_line() {
        // Three rows leaves no room above the status bar for completions.
        let rendered = render_at("theme ", 40, 3);
        assert!(rendered.contains(":theme"), "{rendered}");
        assert!(!rendered.contains("Completions"));
    }

    #[test]
    fn a_cramped_terminal_does_not_panic() {
        for (width, height) in [(8, 3), (20, 4), (12, 10)] {
            let _ = render_at("theme n", width, height);
        }
    }
}
