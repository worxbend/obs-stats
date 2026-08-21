//! The tab strip: which dashboards exist and how they are ordered.

/// One top-level dashboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum View {
    /// btop-style at-a-glance deck: gauges, health verdict, live outputs.
    Overview,
    /// Frame accounting: FPS against target, render budget, skip rates.
    Performance,
    /// Stream and recording outputs in detail, with a bitrate chart.
    Stream,
    /// Scene list, program/preview, profiles and collections.
    Scenes,
    /// Audio inputs with dB faders and live peak meters.
    Audio,
    /// The event stream from OBS.
    Log,
}

impl View {
    /// Every view, in tab order.
    pub const ALL: [View; 6] = [
        View::Overview,
        View::Performance,
        View::Stream,
        View::Scenes,
        View::Audio,
        View::Log,
    ];

    /// Stable id used in config and on the CLI.
    pub fn id(self) -> &'static str {
        match self {
            View::Overview => "overview",
            View::Performance => "performance",
            View::Stream => "stream",
            View::Scenes => "scenes",
            View::Audio => "audio",
            View::Log => "log",
        }
    }

    /// Title shown in the tab strip.
    pub fn title(self) -> &'static str {
        match self {
            View::Overview => "Overview",
            View::Performance => "Performance",
            View::Stream => "Stream",
            View::Scenes => "Scenes",
            View::Audio => "Audio",
            View::Log => "Log",
        }
    }

    /// Glyph prefix, shown when `ui.show_icons` is on.
    ///
    /// Every icon is deliberately a single-column BMP symbol rather than an
    /// emoji: wide glyphs make the tab strip's spacing uneven and throw off
    /// the column arithmetic that maps a mouse click back to a tab.
    pub fn icon(self) -> &'static str {
        match self {
            View::Overview => "▤",
            View::Performance => "◈",
            View::Stream => "◉",
            View::Scenes => "▦",
            View::Audio => "♪",
            View::Log => "≡",
        }
    }

    /// The tab-strip label: the icon, a space, and the title — or just the
    /// title when icons are off.
    ///
    /// Both the renderer and mouse hit-testing go through this, so the strip
    /// cannot be drawn one way and clicked another. It excludes the `"N "`
    /// number prefix, which `ui::render_tabs` adds as its own span.
    pub fn tab_label(self, show_icons: bool) -> String {
        if show_icons {
            format!("{} {}", self.icon(), self.title())
        } else {
            self.title().to_string()
        }
    }

    /// One-line description, shown in the help overlay.
    pub fn description(self) -> &'static str {
        match self {
            View::Overview => "Health verdict, CPU/memory/FPS gauges, live output cards",
            View::Performance => "Frame pacing: FPS vs target, render budget, skip rates",
            View::Stream => "Stream and recording detail with a live bitrate chart",
            View::Scenes => "Scene list with program/preview, profiles and collections",
            View::Audio => "Audio inputs with faders and live peak meters",
            View::Log => "Events reported by OBS since this session started",
        }
    }

    pub fn from_id(id: &str) -> Option<View> {
        let wanted = id.trim().to_ascii_lowercase();
        View::ALL.into_iter().find(|view| view.id() == wanted)
    }

    /// Position in the tab strip, 0-based.
    pub fn index(self) -> usize {
        View::ALL.iter().position(|view| *view == self).unwrap_or(0)
    }

    /// The view at `index`, or `None` when out of range.
    pub fn from_index(index: usize) -> Option<View> {
        View::ALL.get(index).copied()
    }

    /// The next tab, wrapping at the end.
    pub fn next(self) -> View {
        View::ALL[(self.index() + 1) % View::ALL.len()]
    }

    /// The previous tab, wrapping at the start.
    pub fn previous(self) -> View {
        View::ALL[(self.index() + View::ALL.len() - 1) % View::ALL.len()]
    }

    /// Whether this view has a selectable list, so the shell knows when
    /// `j`/`k` should move a cursor rather than doing nothing.
    pub fn is_scrollable(self) -> bool {
        matches!(self, View::Scenes | View::Audio | View::Log)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn ids_and_titles_are_unique() {
        let ids: HashSet<&str> = View::ALL.iter().map(|view| view.id()).collect();
        let titles: HashSet<&str> = View::ALL.iter().map(|view| view.title()).collect();
        assert_eq!(ids.len(), View::ALL.len());
        assert_eq!(titles.len(), View::ALL.len());
    }

    #[test]
    fn every_view_round_trips_through_its_id_and_index() {
        for view in View::ALL {
            assert_eq!(View::from_id(view.id()), Some(view));
            assert_eq!(View::from_index(view.index()), Some(view));
        }
    }

    #[test]
    fn id_lookup_is_forgiving_about_case_and_padding() {
        assert_eq!(View::from_id("  OVERVIEW "), Some(View::Overview));
        assert_eq!(View::from_id("Audio"), Some(View::Audio));
        assert_eq!(View::from_id("nope"), None);
        assert_eq!(View::from_index(99), None);
    }

    #[test]
    fn tabs_wrap_in_both_directions() {
        assert_eq!(View::Overview.previous(), View::Log);
        assert_eq!(View::Log.next(), View::Overview);
        assert_eq!(View::Overview.next(), View::Performance);
        assert_eq!(View::Performance.previous(), View::Overview);
    }

    #[test]
    fn stepping_all_the_way_round_returns_to_the_start() {
        let mut view = View::Overview;
        for _ in 0..View::ALL.len() {
            view = view.next();
        }
        assert_eq!(view, View::Overview);
    }

    #[test]
    fn only_list_views_scroll() {
        assert!(View::Scenes.is_scrollable());
        assert!(View::Audio.is_scrollable());
        assert!(View::Log.is_scrollable());
        assert!(!View::Overview.is_scrollable());
        assert!(!View::Performance.is_scrollable());
        assert!(!View::Stream.is_scrollable());
    }

    #[test]
    fn the_default_start_view_exists() {
        let default_view = crate::config::UiConfig::default().start_view;
        assert!(
            View::from_id(&default_view).is_some(),
            "{default_view} is missing"
        );
    }

    #[test]
    fn descriptions_and_icons_are_present() {
        for view in View::ALL {
            assert!(
                !view.description().is_empty(),
                "{} has no description",
                view.id()
            );
            assert!(!view.icon().is_empty(), "{} has no icon", view.id());
        }
    }
}
