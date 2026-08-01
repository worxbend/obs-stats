//! The Scenes view — scene list plus the profile and collection context.
//!
//! `obs-stats` is a read-only monitor, so the list is a cursor, not a control:
//! selecting a scene never switches it in OBS.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};

use crate::metrics::Snapshot;
use crate::tui::app::App;
use crate::tui::theme::Theme;
use crate::tui::view::View;
use crate::tui::widgets::chrome;
use crate::util;

pub fn render(frame: &mut Frame, area: Rect, app: &mut App) {
    // Register the row count before taking a shared borrow of the snapshot,
    // so the cursor is clamped against the list we are about to draw.
    app.set_row_count(View::Scenes, app.snapshot.scenes.scenes.len());

    let theme = app.theme;
    let selected = app.selection(View::Scenes);
    let scenes = &app.snapshot.scenes;

    let [list_area, side] =
        Layout::horizontal([Constraint::Min(30), Constraint::Length(34)]).areas(area);

    let block = chrome::panel_with_note(
        "Scenes",
        format!("{}", scenes.scenes.len()),
        true,
        theme,
        app.unicode,
    );
    let inner = block.inner(list_area);
    frame.render_widget(block, list_area);

    if scenes.scenes.is_empty() {
        frame.render_widget(
            Paragraph::new(chrome::placeholder("no scenes reported yet", theme)),
            inner,
        );
    } else {
        let width = inner.width as usize;
        let items: Vec<ListItem> = scenes
            .scenes
            .iter()
            .map(|scene| {
                ListItem::new(scene_line(
                    &scene.name,
                    scene.name == scenes.current_program,
                    scene.name == scenes.current_preview,
                    width,
                    theme,
                    app.unicode,
                ))
            })
            .collect();

        let list = List::new(items).highlight_style(
            Style::default()
                .bg(theme.highlight_bg)
                .fg(theme.highlight_fg)
                .add_modifier(Modifier::BOLD),
        );
        let mut state = ListState::default();
        state.select(Some(selected));
        frame.render_stateful_widget(list, inner, &mut state);
    }

    render_context(frame, side, app);
}

/// One scene row, tagged PROGRAM / PREVIEW where applicable.
pub fn scene_line(
    name: &str,
    is_program: bool,
    is_preview: bool,
    width: usize,
    theme: &Theme,
    unicode: bool,
) -> Line<'static> {
    let glyphs = chrome::glyphs(unicode);
    let (marker, marker_color) = if is_program {
        (glyphs.dot, theme.danger)
    } else if is_preview {
        (glyphs.dot, theme.info)
    } else {
        (glyphs.bullet, theme.border)
    };

    let tag = if is_program {
        Some(("PROGRAM", theme.danger))
    } else if is_preview {
        Some(("PREVIEW", theme.info))
    } else {
        None
    };

    // Reserve room for the marker, the gap, and the tag so a long scene name
    // is truncated rather than pushing the tag off the panel.
    let tag_width = tag.map_or(0, |(text, _)| text.len() + 2);
    let name_width = width.saturating_sub(2 + tag_width).max(4);

    let mut spans = vec![
        Span::styled(format!("{marker} "), Style::default().fg(marker_color)),
        Span::styled(
            util::truncate(name, name_width),
            Style::default()
                .fg(if is_program { theme.fg } else { theme.muted })
                .add_modifier(if is_program {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ),
    ];

    if let Some((text, color)) = tag {
        spans.push(Span::styled(
            format!("  {text}"),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ));
    }

    Line::from(spans)
}

/// The right-hand column: profiles and scene collections.
fn render_context(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme;
    let snapshot = &app.snapshot;

    let [profiles, collections] = Layout::vertical([Constraint::Ratio(1, 2); 2]).areas(area);

    let profile_block = chrome::panel("Profiles", false, theme, app.unicode);
    let profile_inner = profile_block.inner(profiles);
    frame.render_widget(profile_block, profiles);
    frame.render_widget(
        Paragraph::new(name_list(
            &snapshot.profiles,
            &snapshot.current_profile,
            "no profiles reported",
            profile_inner.width as usize,
            theme,
            app.unicode,
        )),
        profile_inner,
    );

    let collection_block = chrome::panel("Collections", false, theme, app.unicode);
    let collection_inner = collection_block.inner(collections);
    frame.render_widget(collection_block, collections);
    frame.render_widget(
        Paragraph::new(name_list(
            &snapshot.collections,
            &snapshot.current_collection,
            "no collections reported",
            collection_inner.width as usize,
            theme,
            app.unicode,
        )),
        collection_inner,
    );
}

/// A list of names with the active one marked.
pub fn name_list(
    names: &[String],
    current: &str,
    empty_message: &str,
    width: usize,
    theme: &Theme,
    unicode: bool,
) -> Vec<Line<'static>> {
    if names.is_empty() {
        return vec![chrome::placeholder(empty_message, theme)];
    }

    let glyphs = chrome::glyphs(unicode);
    names
        .iter()
        .map(|name| {
            let active = name == current;
            Line::from(vec![
                Span::styled(
                    format!("{} ", if active { glyphs.dot } else { glyphs.bullet }),
                    Style::default().fg(if active { theme.accent } else { theme.border }),
                ),
                Span::styled(
                    util::truncate(name, width.saturating_sub(2).max(4)),
                    Style::default()
                        .fg(if active { theme.fg } else { theme.muted })
                        .add_modifier(if active {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                ),
            ])
        })
        .collect()
}

/// Summary line used by the tests and the Log view's context strip.
pub fn context_summary(snapshot: &Snapshot) -> String {
    format!(
        "{} · {} · {}",
        if snapshot.scenes.current_program.is_empty() {
            "—"
        } else {
            &snapshot.scenes.current_program
        },
        if snapshot.current_profile.is_empty() {
            "—"
        } else {
            &snapshot.current_profile
        },
        if snapshot.current_collection.is_empty() {
            "—"
        } else {
            &snapshot.current_collection
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::obs::stats::SceneList;
    use crate::tui::theme;

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    fn joined(lines: &[Line<'_>]) -> String {
        lines.iter().map(text).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn the_program_scene_is_tagged_and_emphasised() {
        let line = scene_line("Live", true, false, 40, theme::FALLBACK, true);
        let rendered = text(&line);
        assert!(rendered.contains("Live"));
        assert!(rendered.contains("PROGRAM"));
        assert!(line.spans[1].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn the_preview_scene_is_tagged_differently() {
        let line = scene_line("Intro", false, true, 40, theme::FALLBACK, true);
        let rendered = text(&line);
        assert!(rendered.contains("PREVIEW"));
        assert!(!rendered.contains("PROGRAM"));
    }

    #[test]
    fn an_ordinary_scene_gets_no_tag() {
        let rendered = text(&scene_line(
            "Outro",
            false,
            false,
            40,
            theme::FALLBACK,
            true,
        ));
        assert!(rendered.contains("Outro"));
        assert!(!rendered.contains("PROGRAM"));
        assert!(!rendered.contains("PREVIEW"));
    }

    #[test]
    fn a_long_scene_name_is_truncated_so_its_tag_survives() {
        let line = scene_line(
            "An Extremely Long Scene Name That Will Not Fit",
            true,
            false,
            30,
            theme::FALLBACK,
            true,
        );
        let rendered = text(&line);
        assert!(rendered.contains('…'));
        assert!(
            rendered.contains("PROGRAM"),
            "the tag must not be pushed off"
        );
    }

    #[test]
    fn an_empty_name_list_shows_its_placeholder() {
        let lines = name_list(&[], "", "no profiles reported", 30, theme::FALLBACK, true);
        assert!(joined(&lines).contains("no profiles reported"));
    }

    #[test]
    fn the_active_name_is_marked_and_bolded() {
        let names = vec!["Twitch".to_string(), "YouTube".to_string()];
        let lines = name_list(&names, "YouTube", "none", 30, theme::FALLBACK, true);
        assert_eq!(lines.len(), 2);
        assert!(
            !lines[0].spans[1]
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
        assert!(
            lines[1].spans[1]
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
        assert_eq!(lines[1].spans[0].content, "● ");
    }

    #[test]
    fn context_summary_falls_back_to_dashes_before_any_data_arrives() {
        assert_eq!(context_summary(&Snapshot::new(8)), "— · — · —");
    }

    #[test]
    fn context_summary_names_the_scene_profile_and_collection() {
        let mut snapshot = Snapshot::new(8);
        snapshot.scenes = SceneList {
            current_program: "Live".into(),
            ..SceneList::default()
        };
        snapshot.current_profile = "Twitch".into();
        snapshot.current_collection = "Main".into();
        assert_eq!(context_summary(&snapshot), "Live · Twitch · Main");
    }
}
