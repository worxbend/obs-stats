//! The frame-loss banner.
//!
//! Sits directly under the tab strip so it is visible from every view — a
//! dropped frame matters whether or not you happen to be looking at the
//! Stream tab. It is deliberately loud: reversed colours across the full
//! width, because the alternative is a subtle indicator nobody notices while
//! they are busy presenting.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::metrics::alerts::{Alert, Severity};
use crate::tui::app::App;
use crate::tui::theme::Theme;
use crate::tui::widgets::chrome;

/// Rows the banner occupies when shown.
pub const HEIGHT: u16 = 1;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let Some(banner) = &app.banner else { return };
    frame.render_widget(
        Paragraph::new(line(
            &banner.alert,
            app.theme,
            app.unicode,
            app.frame,
            area.width,
        )),
        area,
    );
}

/// The banner's single line, styled as a full-width reversed bar.
///
/// `width` is the row it has to fit in. The remedy hint is the first thing
/// dropped when space is short — a half-printed instruction is worse than
/// none, and the cause and count are what must survive.
pub fn line(alert: &Alert, theme: &Theme, unicode: bool, frame: u64, width: u16) -> Line<'static> {
    let colour = match alert.severity() {
        Severity::Critical => theme.danger,
        Severity::Warning => theme.warning,
    };

    // Critical alerts pulse so they read as live rather than as a leftover
    // from five minutes ago. Warnings stay steady — a flashing amber bar for
    // a render hiccup would be worse than the hiccup.
    let emphasised = alert.severity() != Severity::Critical || frame % 20 < 14;
    let base = if emphasised {
        Style::default()
            .bg(colour)
            .fg(theme.highlight_fg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .bg(theme.bg)
            .fg(colour)
            .add_modifier(Modifier::BOLD)
    };

    let marker = match (alert.severity(), unicode) {
        (Severity::Critical, true) => "✖",
        (Severity::Warning, true) => "▲",
        (Severity::Critical, false) => "!!",
        (Severity::Warning, false) => "!",
    };

    let label = format!(" {marker} {} ", alert.kind.label());
    let summary = format!(" {}", alert.summary());
    let hint = format!("  {} {}", chrome::glyphs(unicode).bullet, alert.kind.hint());

    let mut spans = vec![
        Span::styled(label.clone(), base),
        Span::styled(
            summary.clone(),
            Style::default().fg(colour).add_modifier(Modifier::BOLD),
        ),
    ];

    let used = label.chars().count() + summary.chars().count();
    if used + hint.chars().count() <= width as usize {
        spans.push(Span::styled(hint, Style::default().fg(theme.muted)));
    }

    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::alerts::AlertKind;
    use crate::tui::theme;
    use std::time::{Duration, Instant};

    fn alert(kind: AlertKind, frames: u64) -> Alert {
        Alert {
            kind,
            frames,
            window: Duration::from_secs(10),
            at: Instant::now(),
        }
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn the_banner_names_the_cause_the_count_and_the_fix() {
        let rendered = text(&line(
            &alert(AlertKind::NetworkDropped, 12),
            theme::FALLBACK,
            true,
            0,
            200,
        ));
        assert!(rendered.contains("DROPPING FRAMES"));
        assert!(rendered.contains("12 frames"));
        assert!(rendered.contains("viewers are seeing stutter"));
        assert!(rendered.contains("Lower the bitrate"));
    }

    #[test]
    fn severity_picks_the_colour() {
        let palette = theme::FALLBACK;
        let critical = line(&alert(AlertKind::NetworkDropped, 3), palette, true, 0, 200);
        let warning = line(&alert(AlertKind::RendererSkipped, 3), palette, true, 0, 200);
        assert_eq!(critical.spans[0].style.bg, Some(palette.danger));
        assert_eq!(warning.spans[0].style.bg, Some(palette.warning));
    }

    #[test]
    fn critical_banners_pulse_and_warnings_do_not() {
        let palette = theme::FALLBACK;
        let lit = line(&alert(AlertKind::NetworkDropped, 3), palette, true, 0, 200);
        let dim = line(&alert(AlertKind::NetworkDropped, 3), palette, true, 15, 200);
        assert_ne!(
            lit.spans[0].style.bg, dim.spans[0].style.bg,
            "a critical banner should pulse"
        );

        for frame in [0, 15] {
            let warning = line(
                &alert(AlertKind::RendererSkipped, 3),
                palette,
                true,
                frame,
                200,
            );
            assert_eq!(
                warning.spans[0].style.bg,
                Some(palette.warning),
                "a warning banner should stay steady"
            );
        }
    }

    #[test]
    fn ascii_mode_uses_plain_markers() {
        let rendered = text(&line(
            &alert(AlertKind::NetworkDropped, 3),
            theme::FALLBACK,
            false,
            0,
            200,
        ));
        assert!(rendered.contains("!!"));
        assert!(rendered.is_ascii() || !rendered.contains('✖'));
    }

    #[test]
    fn a_reconnect_banner_reads_without_a_frame_count() {
        let rendered = text(&line(
            &alert(AlertKind::Reconnecting, 0),
            theme::FALLBACK,
            true,
            0,
            200,
        ));
        assert!(rendered.contains("RECONNECTING"));
        assert!(rendered.contains("the stream has dropped"));
        assert!(!rendered.contains("0 frames"));
    }

    #[test]
    fn a_narrow_banner_drops_the_hint_rather_than_clipping_it() {
        let wide = text(&line(
            &alert(AlertKind::NetworkDropped, 14),
            theme::FALLBACK,
            true,
            0,
            200,
        ));
        assert!(wide.contains("Lower the bitrate"));

        let narrow = text(&line(
            &alert(AlertKind::NetworkDropped, 14),
            theme::FALLBACK,
            true,
            0,
            70,
        ));
        assert!(!narrow.contains("Lower the bitrate"), "{narrow}");
        // The cause and the count are what must survive.
        assert!(narrow.contains("DROPPING FRAMES"));
        assert!(narrow.contains("14 frames"));
        assert!(narrow.chars().count() <= 70, "{}", narrow.chars().count());
    }

    #[test]
    fn every_kind_renders_a_banner() {
        for kind in AlertKind::ALL {
            let rendered = text(&line(&alert(kind, 4), theme::FALLBACK, true, 0, 200));
            assert!(rendered.contains(kind.label()), "{kind:?}");
            assert!(rendered.contains(kind.hint()), "{kind:?}");
        }
    }
}
