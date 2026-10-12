# Dashboard design and verification contract

## Data honesty before decoration

This is a read-only OBS monitor. A premium layout must make output health, loss
cause, connection state, and measurement scope legible without inventing a richer
telemetry source. Preview fixtures are **synthetic**, not captured production
sessions. The screenshot example's live path now leaves observed output state,
history, and audio levels untouched; it no longer dresses idle OBS with simulated
streaming. No production acquisition or protocol changes are required for previews.

The signature documentation theme is `grafana`. Other built-in themes remain
first-class, including terminal-controlled `mono`. Do not rely on color or icons
alone for severity, selection, connection, or pause. Keep units beside values and
use text labels for the read-only affordance. The overview's hierarchy is four
headline KPIs, network/health analysis, resources, and output details; compact
terminals must retain meaningful labels rather than squeezing the full desktop
layout into undersized cards.

## Existing telemetry inventory

These are fields supported by the current code, not evidence that a particular
production OBS instance was contacted for this redesign.

| Source | Available measurement or context | Meaning / limit |
| --- | --- | --- |
| `GetStats` | CPU usage, memory MiB, free recording-volume space MiB, active FPS, mean render ms | OBS process/render data; not host-wide CPU, GPU, or network metrics |
| `GetStats` | Render skipped/total, encoder skipped/total | Since OBS launch; the model captures a stream baseline and subtracts it when available |
| `GetStats` | Websocket incoming/outgoing message counts | Session message counters, not latency or throughput |
| `GetStreamStatus` | Active, reconnecting, elapsed/timecode, bytes, congestion, network dropped/total | Network counters are already per-stream; congestion is a `0..1` fraction |
| `GetRecordStatus` | Active, paused, elapsed/timecode, bytes | Recording output state; no encoder settings or recording filename in the current model |
| `GetVideoSettings` | Canvas/output dimensions and rational target FPS | Settings context, not achieved frame rate |
| Inventory requests | Program/preview scene, scene list, profiles/collections and their current names, audio input name/kind | Audio kinds are filtered by the existing codec; scene selection never switches OBS |
| Input mute/volume requests | Mute state and fader dB | Missing halves retain their earlier values; a fader is not an instantaneous peak |
| `InputVolumeMeters` | Linear peak per channel | Event-driven meters; empty/muted or not-yet-received levels must not become invented sound |
| OBS events and session task | Human-readable events, connection attempts, offline/fatal reason and hint, retry interval | Application/OBS event journal, not the full OBS log |
| `Snapshot` derived values | Byte-delta stream/write rates, session-relative skips, FPS/frame-budget ratios, health, ring histories, per-cause rate alerts | First-rate sample is zero; restart/backwards counters do not create negative rates; health is a local rule, not OBS's verdict |
| Application state | Last poll age, pause, alert count/banner, selected view/row, theme, command and key-prefix state | Connection updates remain visible while measurement display is paused |

**Not acquired:** CPU cores/load, GPU/VRAM/temperature, ping/RTT, host traffic,
packet loss, dropped-frame root cause beyond the three reported counters,
encoder/codec/preset, service destination, viewers/chat, filesystem identity,
scene thumbnails, per-source resource cost, loudness/LUFS, clipping duration, or
an archival telemetry database. Do not place plausible placeholder values for
these in a production-looking dashboard.

**Missing versus zero:** OBS decoders are deliberately total and may default a
missing/failed numeric field to zero. There is no general per-field validity bit
in `Snapshot`. Global poll availability is distinguishable (`last_poll`, `polls`,
history), but a UI-only redesign cannot prove every zero is an observed reading.
Disconnect retains counters/context but zeros derived rates and clears peaks.
It also clears stream baselines; skip counters then fall back to since-launch
scope until a successful stream poll re-baselines them.

## Architecture boundaries

- The websocket session owns reconnect/backoff and acquisition. Hot requests are
  batched; inventory runs every ten hot ticks, followed by audio fader requests.
  Updates pass through a bounded channel. No view performs network or disk I/O.
- The codec builds/decodes requests without I/O. The model derives rates,
  baselines, histories and alerts from typed updates. Preview conditions feed
  `Update::Poll` through this same model, rather than replacing health labels.
- `App` handles actions and records current-frame hit regions. Key resolution is
  pure; the command registry is shared by palette and command line. Preserve
  keyboard and mouse contracts while moving panels.
- `tui::run` owns terminal lifecycle and event scheduling. Deterministic
  TestBackend capture verifies cells, not raw-mode cleanup or actual OBS behavior.

Source: [`snapshot.rs`](../src/metrics/snapshot.rs),
[`codec.rs`](../src/obs/codec.rs), [`stats.rs`](../src/obs/stats.rs),
[`client.rs`](../src/obs/client.rs), [`run.rs`](../src/tui/run.rs),
[`cli.rs`](../src/cli.rs), and [`probe.rs`](../src/probe.rs).

## Deterministic export interface

The existing CI entry point and image filenames remain intact:

```sh
cargo run --example screenshot -- --demo
```

This still renders the twelve documentation shots at `132x38` into
`docs/images/*.svg`. It includes all six views and overlay/key-prefix examples,
with their configured shot themes. The `PLAIN` shots now use `grafana` for the
signature redesign. The documentation drift gate is unchanged; regenerate
published images only after the renderer is complete.

Any scenario, size, view, shot, theme, matrix, or raw-cell selection activates
**synthetic preview mode without contacting OBS**, even without `--demo`.
Previews default to `docs/previews`, never replace documentation image names,
and include scenario/theme/geometry in filenames and synthetic provenance in
SVG descriptions, text headers, and cell JSON. `--output DIR` can relocate
exports; with only `--demo --output DIR`, the legacy fixture and filenames are
preserved for scratch comparisons. No installed config or credentials are read
by synthetic mode.

```sh
# One compact healthy frame; repeat --size/--scenario/--view/--theme as needed.
cargo run --example screenshot -- --view overview --size 80x24 --scenario healthy --text

# Accessible semantic output: no ANSI sequences, colors, or drawing glyphs.
cargo run --example screenshot -- --view overview --scenario offline --text summary

# Nine conditions x five sizes x all twelve documentation shots.
cargo run --example screenshot -- --matrix --output "$TMPDIR/obs-stats-preview"

# All built-in themes; constrain a large sweep to one view if desired.
cargo run --example screenshot -- --matrix --all-themes --view overview --output "$TMPDIR/obs-stats-themes"

# Preserve actual cell foreground/background/modifiers, including Reset.
cargo run --example screenshot -- --scenario drops --theme grafana --size 132x38 --cells --output "$TMPDIR/obs-stats-cells"

# Fallback drawing: input/scene/event text itself can still contain Unicode.
cargo run --example screenshot -- --matrix --ascii --text grid
```

Selections are validated; dimensions are bounded to `1x1..400x160`; duplicate
selections do not inflate the output count. `--view` selects plain views;
`--shot` selects the named documentation shot, including overlays. These two
selectors are mutually exclusive. With no theme override, each selected shot
retains its own theme. Explicit sizes/scenarios narrow the corresponding matrix
axis. File/directory write errors exit unsuccessfully instead of claiming a
partially written set succeeded.

### Conditions

| Scenario | Fixture state |
| --- | --- |
| `healthy` | Connected; 256 modeled polls; 60 FPS target; low render cost/congestion; populated histories and peaks |
| `strained` | Connected; reduced FPS, elevated CPU/render cost/congestion and render skips; model returns STRAINED |
| `drops` | Connected; network/encoder losses, high congestion, low recording space; model returns DROPPING and a synthetic loss banner is visible |
| `loading` | First connection attempt; no poll, counters, inventory, history, or peaks received |
| `offline` | Disconnect after prior samples; context/history retained; rates zero and meters cleared by the model |
| `retrying` | Same retained data after connection loss; connection attempt three |
| `paused` | Display pause set; captured poll age thirty seconds; distinct from OBS recording pause |
| `recording-paused` | OBS recording remains active but paused; derived write rate zero; dashboard itself not paused |
| `stale` | Connected with a last poll sixty seconds old; this is not a fresh healthy observation |

The matrix sizes are `60x18`, `80x24`, `100x30`, `132x38`, and `180x50`.
Example tests also exercise `1x1` and `19x7`. Synthetic counter baselines and rates
come from fixed one-second poll steps; wave-derived CPU/memory/frame-time samples
use no RNG. Fixture logs have fixed relative timestamps. The shown version,
blink frame, and legacy poll count are pinned; populated legacy
demo frames have a last poll one second before the supplied capture instant, so
they do not claim to be warming up. `ui::draw_at` receives that same reference
instant, freezing rendering time as well as sample time. A silent connected
socket becomes visibly `STALE` after three poll intervals (at least three
seconds); every view shows the last-sample age, and retained values cannot claim
current `LIVE` or `HEALTHY` state. Pausing is explicit and is not treated as an
acquisition failure.

## Evidence and release verification

1. Run `cargo test --example screenshot --locked` explicitly: example-local tests
   are not executed by the ordinary `cargo test` gate. They check parser/default
   contracts, coherent health/rates, unavailable-data semantics, raw Reset and
   coordinates, repeated captures one hour apart, every view/theme, and
   scenario/size/shot/ASCII combinations.
2. Run the repository's format, all-target/all-feature clippy and ordinary test
   gates. Re-render `--demo` after implementation; a second identical run must
   leave no image diff. Compare scratch legacy exports before touching published
   images when investigating generator-only changes.
3. Review actual cells for theme holes, clipped labels/units, selected rows,
   overlays, key-prefix help, and narrow-terminal connection severity. SVG's
   historical Reset mapping resolves to theme colors and can hide transparent
   cells; inspect `--cells` and keep the existing raw-buffer background tests.
4. Compare two complete export runs by filenames and SHA-256, not by visual
   similarity. SVGs describe real cells but use viewer-installed fonts; this is
   not a pinned-font/browser pixel-reproducibility claim. Indexed colors retain
   the legacy SVG approximation; cell JSON records the exact original index.
5. Exercise a real PTY/terminal for keyboard input, resize, rendering cadence,
   overlay dismissal, mouse targets and cleanup. Fake transport scenarios can
   establish realistic connection failure behavior but are still synthetic.
   Live OBS checks require an actual service and must be reported separately.

A no-panic matrix and matching SVG bytes are necessary regression evidence, not
proof of premium visual quality, full terminal compatibility, or production
telemetry acquisition.
