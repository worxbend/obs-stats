# Changelog

## 0.6.0 — 2026-10-13

### Dashboard

- Introduce the signature Grafana dark palette: deep ink surfaces, orange/cyan accents, violet recording signals, and consistent semantic severity colors. All existing themes remain available.
- Rebuild Overview around headline metrics, retained-history throughput analytics, rolling averages/peaks, a backpressure heat strip, resource cards, and stream/recording summaries.
- Refresh Performance, Stream, Scenes, Audio, and Log with metric headlines, routing/event inspectors, numeric audio peaks, contextual information, and compact layouts.
- Apply the same visual language to panels, tabs, theme previews, help, command completion, palette, key-prefix menus, and status hints.
- Show initial connection and unavailable-data states without invented readings. Mark disconnected or silently stale telemetry explicitly; paused displays no longer claim current live output.

### Reliability and verification

- Coalesce meter/stat bursts at the 10 Hz render cadence while retaining immediate keyboard and resize responses.
- Stop consuming closed session channels repeatedly and preserve the original terminal connection error.
- Plot the retained observation window in Unicode and ASCII modes, preserving historical spikes. Chart axes label sample distance instead of implying timestamps that were not collected.
- Add an explicit rendering clock for deterministic stale-age previews and broad theme/view/state/background/ASCII regression coverage.
- Expand deterministic previews with terminal sizes, streaming/error/pause scenarios, raw cell exports, and explicit synthetic-fixture provenance. Live previews no longer dress observed OBS state with simulated streaming.
- Add repeatable real-PTY smoke checks and strengthen CI/release quality gates, version validation, archive checks, and checksum verification.

OBS acquisition, the read-only interaction model, frame-loss alert policy, secret handling, and the public `once` JSON contract are preserved. Encoder type and configured target bitrate remain unavailable rather than guessed.

Earlier releases: <https://github.com/worxbend/obs-stats/releases>.
