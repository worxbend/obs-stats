//! Small formatting helpers shared by the views.
//!
//! These are deliberately allocation-light and never panic on NaN/inf, because
//! they run on every metric on every frame.

/// Format a byte count with binary units (`1.5 GiB`).
pub fn bytes(value: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    let mut scaled = value as f64;
    let mut unit = 0;
    while scaled >= 1024.0 && unit < UNITS.len() - 1 {
        scaled /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{value} B")
    } else {
        format!("{scaled:.1} {}", UNITS[unit])
    }
}

/// Format a bit rate in kbps/Mbps, the units streamers actually think in.
pub fn bitrate(bits_per_second: f64) -> String {
    if !bits_per_second.is_finite() || bits_per_second <= 0.0 {
        return "0 kbps".to_string();
    }
    let kbps = bits_per_second / 1000.0;
    if kbps >= 1000.0 {
        format!("{:.2} Mbps", kbps / 1000.0)
    } else {
        format!("{kbps:.0} kbps")
    }
}

/// Format a duration in milliseconds as `H:MM:SS`, or `MM:SS` under an hour.
pub fn duration_ms(millis: u64) -> String {
    let total_seconds = millis / 1000;
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

/// Format a percentage, clamped to `0..=100` so a transient overshoot in a
/// derived rate never renders as `-4%` or `312%`.
pub fn percent(value: f64) -> String {
    format!("{:.1}%", clamp_percent(value))
}

/// Clamp a percentage into `0.0..=100.0`, mapping NaN to zero.
pub fn clamp_percent(value: f64) -> f64 {
    if value.is_nan() {
        0.0
    } else {
        value.clamp(0.0, 100.0)
    }
}

/// Format a frame render time with sub-millisecond precision.
///
/// OBS reports `averageFrameRenderTime` in milliseconds already, so this only
/// fixes the precision and guards against the `0.0` OBS sends while idle.
pub fn frame_time(millis: f64) -> String {
    if !millis.is_finite() || millis <= 0.0 {
        return "0.00 ms".to_string();
    }
    format!("{millis:.2} ms")
}

/// Truncate to `width` display columns, appending `…` when it does not fit.
///
/// Operates on `char`s rather than bytes so multi-byte scene names never get
/// sliced mid-codepoint.
pub fn truncate(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let count = text.chars().count();
    if count <= width {
        return text.to_string();
    }
    if width == 1 {
        return "…".to_string();
    }
    let mut out: String = text.chars().take(width - 1).collect();
    out.push('…');
    out
}

/// Convert a linear audio level (0.0–1.0) to dBFS, with a -60 dB floor.
pub fn linear_to_db(level: f32) -> f32 {
    if level < 1e-7 {
        -60.0
    } else {
        (20.0 * level.log10()).max(-60.0)
    }
}

/// A ratio in `0.0..=1.0` describing how far `value` sits between `min` and
/// `max`. Returns 0.0 for a degenerate range rather than dividing by zero.
pub fn ratio(value: f64, min: f64, max: f64) -> f64 {
    if !value.is_finite() || (max - min).abs() < f64::EPSILON {
        return 0.0;
    }
    ((value - min) / (max - min)).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_scales_by_binary_units() {
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(1024), "1.0 KiB");
        assert_eq!(bytes(1024 * 1024 * 3 / 2), "1.5 MiB");
        assert_eq!(bytes(1024u64.pow(3)), "1.0 GiB");
    }

    #[test]
    fn bitrate_switches_to_mbps_at_a_thousand_kbps() {
        assert_eq!(bitrate(0.0), "0 kbps");
        assert_eq!(bitrate(-5.0), "0 kbps");
        assert_eq!(bitrate(f64::NAN), "0 kbps");
        assert_eq!(bitrate(6_000_000.0), "6.00 Mbps");
        assert_eq!(bitrate(2_500_000.0), "2.50 Mbps");
        assert_eq!(bitrate(850_000.0), "850 kbps");
    }

    #[test]
    fn duration_drops_the_hour_field_under_an_hour() {
        assert_eq!(duration_ms(0), "00:00");
        assert_eq!(duration_ms(65_000), "01:05");
        assert_eq!(duration_ms(3_600_000), "1:00:00");
        assert_eq!(duration_ms(7_384_000), "2:03:04");
    }

    #[test]
    fn percent_clamps_out_of_range_and_nan() {
        assert_eq!(percent(42.31), "42.3%");
        assert_eq!(percent(-1.0), "0.0%");
        assert_eq!(percent(140.0), "100.0%");
        assert_eq!(clamp_percent(f64::NAN), 0.0);
    }

    #[test]
    fn frame_time_formats_millis_and_guards_non_finite() {
        assert_eq!(frame_time(0.0), "0.00 ms");
        assert_eq!(frame_time(1.5), "1.50 ms");
        assert_eq!(frame_time(12.345), "12.35 ms");
        assert_eq!(frame_time(f64::INFINITY), "0.00 ms");
        assert_eq!(frame_time(f64::NAN), "0.00 ms");
    }

    #[test]
    fn truncate_respects_char_boundaries() {
        assert_eq!(truncate("scene", 10), "scene");
        assert_eq!(truncate("scene", 5), "scene");
        assert_eq!(truncate("scene", 4), "sce…");
        assert_eq!(truncate("scene", 1), "…");
        assert_eq!(truncate("scene", 0), "");
        // Multi-byte input must not be sliced mid-codepoint.
        assert_eq!(truncate("камера", 4), "кам…");
    }

    #[test]
    fn linear_to_db_floors_at_minus_sixty() {
        assert_eq!(linear_to_db(0.0), -60.0);
        assert_eq!(linear_to_db(1e-9), -60.0);
        assert!((linear_to_db(1.0) - 0.0).abs() < 1e-5);
        assert!((linear_to_db(0.1) + 20.0).abs() < 1e-4);
    }

    #[test]
    fn ratio_handles_degenerate_ranges() {
        assert_eq!(ratio(5.0, 0.0, 10.0), 0.5);
        assert_eq!(ratio(50.0, 0.0, 10.0), 1.0);
        assert_eq!(ratio(-5.0, 0.0, 10.0), 0.0);
        assert_eq!(ratio(5.0, 3.0, 3.0), 0.0);
        assert_eq!(ratio(f64::NAN, 0.0, 10.0), 0.0);
    }
}
