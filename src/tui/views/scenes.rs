//! The Scenes view — scene list plus the profile and collection context.
//!
//! `obs-stats` is a read-only monitor, so the list is a cursor, not a control:
//! selecting a scene never switches it in OBS.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::metrics::Snapshot;
use crate::tui::app::App;
use crate::tui::theme::Theme;
use crate::tui::view::View;
use crate::tui::widgets::chrome;
use crate::tui::widgets::list;
use crate::util;

pub fn render(frame: &mut Frame, area: Rect, app: &mut App) {
    // Register the row count before taking a shared borrow of the snapshot,
    // so the cursor is clamped against the list we are about to draw.
    app.set_row_count(View::Scenes, app.snapshot.scenes.scenes.len());
    // The scroll state lives in the app so the offset the list settles on
    // survives the frame, which is what a click and the scrollbar resolve
    // against. It is taken out rather than borrowed because the rows below
    // hold a shared borrow of the snapshot.
    let mut state = app.take_list_state(View::Scenes);

    let theme = app.theme;
    let selected = app.selection(View::Scenes);
    let scenes = &app.snapshot.scenes;

    let [routing, body, hint] = Layout::vertical([
        Constraint::Length(4),
        Constraint::Min(4),
        Constraint::Length(1),
    ])
    .areas(area);
    render_routing(frame, routing, app);
    frame.render_widget(
        Paragraph::new(chrome::placeholder(
            "Read-only / selection inspects a scene; it never switches OBS",
            theme,
        )),
        hint,
    );
    let [list_area, side] = if area.width >= 90 {
        Layout::horizontal([Constraint::Min(30), Constraint::Length(36)]).areas(body)
    } else {
        Layout::vertical([Constraint::Min(4), Constraint::Length(6)]).areas(body)
    };

    let block = chrome::panel_with_note(
        "Scenes",
        format!("{} scenes / inspect", scenes.scenes.len()),
        true,
        theme,
        app.unicode,
    );
    let inner = chrome::frame_panel(frame, list_area, block);

    if scenes.scenes.is_empty() {
        frame.render_widget(
            Paragraph::new(chrome::placeholder("no scenes reported yet", theme)),
            inner,
        );
    } else {
        let width = inner.width as usize;
        let items: Vec<Line> = scenes
            .scenes
            .iter()
            .map(|scene| {
                scene_line(
                    &scene.name,
                    scene.name == scenes.current_program,
                    scene.name == scenes.current_preview,
                    width,
                    theme,
                    app.unicode,
                )
            })
            .collect();

        list::render_rows(
            frame,
            inner,
            items,
            selected,
            &mut state,
            theme,
            app.unicode,
        );
    }

    app.put_list_state(View::Scenes, inner, state);
    render_context(frame, side, app, area.width >= 90);
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
        spans.push(Span::raw(" "));
        spans.extend(chrome::badge(text, color, theme));
    }

    Line::from(spans)
}

fn render_routing(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme;
    let [program, preview] = Layout::horizontal([Constraint::Ratio(1, 2); 2]).areas(area);
    for (rect, title, name, color) in [
        (
            program,
            "Program",
            &app.snapshot.scenes.current_program,
            theme.danger,
        ),
        (
            preview,
            "Preview",
            &app.snapshot.scenes.current_preview,
            theme.info,
        ),
    ] {
        let inner = chrome::frame_panel(
            frame,
            rect,
            chrome::panel_with_note(title, "last reported", false, theme, app.unicode),
        );
        let value = if name.is_empty() {
            "not reported"
        } else {
            name
        };
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    util::truncate(value, inner.width as usize),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    if title == "Program" {
                        "current output scene"
                    } else {
                        "reported preview scene"
                    },
                    Style::default().fg(theme.muted),
                )),
            ]),
            inner,
        );
    }
}

/// Inspector and OBS inventory; narrow screens keep the active names visible.
fn render_context(frame: &mut Frame, area: Rect, app: &App, wide: bool) {
    let theme = app.theme;
    let snapshot = &app.snapshot;

    let [profiles, collections] = if wide {
        let [inspection, names] =
            Layout::vertical([Constraint::Length(7), Constraint::Min(4)]).areas(area);
        let inner = chrome::frame_panel(
            frame,
            inspection,
            chrome::panel_with_note("Selected scene", "inspect only", false, theme, app.unicode),
        );
        frame.render_widget(
            Paragraph::new(selected_scene(snapshot, app.selection(View::Scenes), theme))
                .wrap(Wrap { trim: false }),
            inner,
        );
        Layout::vertical([Constraint::Ratio(1, 2); 2]).areas(names)
    } else {
        Layout::horizontal([Constraint::Ratio(1, 2); 2]).areas(area)
    };

    let profile_block = chrome::panel_with_note(
        "Profiles",
        format!("{} reported", snapshot.profiles.len()),
        false,
        theme,
        app.unicode,
    );
    let profile_inner = chrome::frame_panel(frame, profiles, profile_block);
    frame.render_widget(
        Paragraph::new(context_names(
            &snapshot.profiles,
            &snapshot.current_profile,
            "no profiles reported",
            profile_inner.width as usize,
            theme,
            app.unicode,
        )),
        profile_inner,
    );

    let collection_block = chrome::panel_with_note(
        "Collections",
        format!("{} reported", snapshot.collections.len()),
        false,
        theme,
        app.unicode,
    );
    let collection_inner = chrome::frame_panel(frame, collections, collection_block);
    frame.render_widget(
        Paragraph::new(context_names(
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

fn selected_scene(snapshot: &Snapshot, selected: usize, theme: &Theme) -> Vec<Line<'static>> {
    let Some(scene) = snapshot.scenes.scenes.get(selected) else {
        return vec![chrome::placeholder("no scene selected", theme)];
    };
    vec![
        Line::from(Span::styled(
            scene.name.clone(),
            Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
        )),
        chrome::row("OBS index", scene.index.to_string(), theme.muted, 12, theme),
        chrome::row(
            "Program",
            if scene.name == snapshot.scenes.current_program {
                "yes"
            } else {
                "no"
            },
            theme.fg,
            12,
            theme,
        ),
        chrome::row(
            "Preview",
            if scene.name == snapshot.scenes.current_preview {
                "yes"
            } else {
                "no"
            },
            theme.fg,
            12,
            theme,
        ),
    ]
}

// Put the current name first so a clipped context panel never hides it.
fn context_names(
    names: &[String],
    current: &str,
    empty: &str,
    width: usize,
    theme: &Theme,
    unicode: bool,
) -> Vec<Line<'static>> {
    let mut ordered = names.to_vec();
    ordered.sort_by_key(|name| name != current);
    if !current.is_empty() && !ordered.iter().any(|name| name == current) {
        ordered.insert(0, current.to_string());
    }
    name_list(&ordered, current, empty, width, theme, unicode)
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

    fn render_text(app: &mut App, width: u16, height: u16) -> String {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), app))
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn routing_inspector_and_click_regions_preserve_read_only_selection() {
        for width in [80, 120] {
            let mut app = App::new(crate::config::Config::default());
            app.view = View::Scenes;
            app.snapshot.scenes.scenes = vec![
                crate::obs::stats::Scene {
                    name: "First scene".into(),
                    index: 7,
                },
                crate::obs::stats::Scene {
                    name: "Second scene".into(),
                    index: 2,
                },
            ];
            app.snapshot.scenes.current_program = "First scene".into();
            let text = render_text(&mut app, width, 30);
            assert!(text.contains("Read-only"));
            assert!(text.contains("Program"));
            let region = app.regions.list.unwrap();
            let row = text.lines().nth(region.y as usize).unwrap();
            assert!(
                row.contains("First scene"),
                "registered geometry points at the actual row"
            );
            assert!(app.click_row(region.y + 1));
            assert_eq!(app.selection(View::Scenes), 1);
            assert_eq!(app.snapshot.scenes.current_program, "First scene");
        }
    }

    #[test]
    fn active_context_is_visible_before_other_inventory_names() {
        let names = vec!["Other".into(), "Current".into()];
        let rows = context_names(&names, "Current", "none", 32, theme::FALLBACK, false);
        assert!(line_text(&rows[0]).contains("Current"));
        let rows = context_names(&[], "Known current", "none", 32, theme::FALLBACK, false);
        assert!(line_text(&rows[0]).contains("Known current"));
    }

    use crate::obs::stats::SceneList;
    use crate::tui::testing::{line_text, lines_text};
    use crate::tui::theme;

    #[test]
    fn the_program_scene_is_tagged_and_emphasised() {
        let line = scene_line("Live", true, false, 40, theme::FALLBACK, true);
        let rendered = line_text(&line);
        assert!(rendered.contains("Live"));
        assert!(rendered.contains("PROGRAM"));
        assert!(line.spans[1].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn the_preview_scene_is_tagged_differently() {
        let line = scene_line("Intro", false, true, 40, theme::FALLBACK, true);
        let rendered = line_text(&line);
        assert!(rendered.contains("PREVIEW"));
        assert!(!rendered.contains("PROGRAM"));
    }

    #[test]
    fn an_ordinary_scene_gets_no_tag() {
        let rendered = line_text(&scene_line(
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
        let rendered = line_text(&line);
        assert!(rendered.contains('…'));
        assert!(
            rendered.contains("PROGRAM"),
            "the tag must not be pushed off"
        );
    }

    #[test]
    fn an_empty_name_list_shows_its_placeholder() {
        let lines = name_list(&[], "", "no profiles reported", 30, theme::FALLBACK, true);
        assert!(lines_text(&lines).contains("no profiles reported"));
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
