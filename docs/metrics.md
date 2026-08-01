# Reading the metrics

Every number on the dashboard comes from obs-websocket. This page explains what
each one means, when to worry, and what usually fixes it.

## The health verdict

The Overview card reduces everything to one word. It is not a single threshold —
it is the worst thing currently true.

| Verdict | Means |
|---|---|
| `HEALTHY` | Nothing below any warning threshold. |
| `STRAINED` | OBS is keeping up, but without much margin. Worth watching; not yet visible to viewers. |
| `DROPPING` | Something is being lost that a viewer can see. |

`DROPPING` is set when any of these is true:

- Network drop rate ≥ 1% of frames sent
- Encoder skip rate ≥ 1% of frames encoded **this stream**
- The stream is reconnecting

`STRAINED` is set when any of these is true:

- Encoder congestion ≥ 30%
- Achieved FPS below 95% of the configured target
- Average frame render time ≥ 50% of the frame budget
- Renderer skip rate ≥ 0.5% **this stream**

## The three ways a frame is lost

OBS's own stats dock tends to blur these together. They have different causes
and different fixes, so `obs-stats` always reports them separately.

### Renderer skipped — *"lagged frames"*

The compositor could not build a frame in time. **This is a GPU problem.**

Causes: too many sources, heavy filters or shaders, a browser source doing real
work, a GPU already busy with the game you are streaming.

Fixes: lower the base (canvas) resolution, cut the frame rate, remove or
simplify filters, cap the game's frame rate to leave the GPU headroom.

### Encoder skipped — *"skipped frames due to encoding lag"*

The encoder could not keep up and dropped a frame before it reached the network.
**This is a CPU or encoder problem.**

Causes: x264 on a preset too slow for the CPU, several encoding sessions at
once, a CPU that is also running the game.

Fixes: switch to a hardware encoder (NVENC, QuickSync, AMF), use a faster x264
preset, lower the output resolution.

### Network dropped

Frames were encoded but could not be delivered. **This is an uplink problem.**

Causes: not enough upload bandwidth, congestion, packet loss, a struggling
ingest server.

Fixes: lower the bitrate, enable dynamic bitrate in OBS, try a closer ingest
endpoint, check for anything else saturating the connection.

Watch **congestion** as the leading indicator: it climbs before frames are
actually dropped, which is the moment to act.

## Why counters reset when you go live

OBS reports `renderSkippedFrames` and `outputSkippedFrames` as totals **since
OBS launched**. After an eight-hour session those totals tell you almost
nothing about the stream you are on now.

`obs-stats` captures a baseline the moment a stream starts and reports the
difference. The Performance panel says which mode it is in — `this stream` or
`since OBS started` — so the number is never ambiguous.

Network drops, from `GetStreamStatus`, are already per-stream in OBS itself.

## Frame pacing

### FPS against target

`activeFps` is what the renderer is achieving. The target comes from
`GetVideoSettings` (`fpsNumerator / fpsDenominator`), so NTSC rates like
59.94 are handled exactly rather than rounded to 60.

The gauge shows the *ratio*. Below 95% is amber, below 85% is red.

### Frame render time against budget

At 60 fps the budget is 16.67 ms; at 30 fps it is 33.3 ms. The gauge shows what
share of that budget the average frame is consuming.

- Under 50% — comfortable.
- 50–80% — strained. A spike will start costing frames.
- Over 80% — you are on the edge; renderer skips are imminent.

This is the most useful early-warning number on the dashboard, because it moves
*before* anything is dropped.

## Outputs

| Field | Source | Notes |
|---|---|---|
| Bitrate | Derived | `Δbytes × 8 ÷ Δt` between polls, not a configured value. What is actually going out. |
| Sent / On disk | `outputBytes` | Cumulative for the current output session. |
| Congestion | `outputCongestion` | 0–1 from OBS, shown as a percentage. Encoder backpressure. |
| Timecode | `outputTimecode` | OBS's own clock; falls back to the duration if absent. |
| Remaining | Derived | Free disk ÷ current write rate. Blank until a rate is measurable; capped at 99 hours. |

Bitrate is derived rather than read because the configured bitrate is a
*request*. Under congestion, or with dynamic bitrate enabled, what actually
leaves the machine is lower — and that difference is the thing worth seeing.

The chart's y-axis windows in on the observed range rather than anchoring at
zero: a stream oscillating between 5.4 and 6.5 Mbps is a flat line at the top of
a 0–7 Mbps axis, and the variation is the whole point.

## System

| Field | Notes |
|---|---|
| CPU | The **OBS process**, as a percentage of one core-equivalent — not system-wide. Amber at 50%, red at 80%. |
| MEM | OBS's resident memory. |
| DISK | Free space on the recording volume. Amber under 20 GiB, red under 5 GiB. |
| WS I/O | Messages exchanged on this WebSocket session. A rough measure of what `obs-stats` itself costs. |

The poll counter beside the connection chip (`#1234`) is the honest liveness
signal: a green chip alone would survive a silently wedged socket, but a poll
counter that stops advancing will not.

## Audio

Meters show **peak** level per channel, from `InputVolumeMeters` events rather
than polling, so they update at OBS's own rate.

The bar is drawn on a dB scale with a −60 dBFS floor. On a linear scale, speech
at −20 dBFS would fill about 10% of the bar and everything would look silent;
on the dB scale it fills roughly two thirds.

- Green through cyan — healthy.
- Amber past about −9 dBFS — getting hot.
- Red past about −3 dBFS — close to clipping.

The number on the right is the loudest recent peak in dBFS. `—` means silence or
a muted input. The value beside the name is the **fader position**, which is a
setting, not a measurement — an input can sit at +0.0 dB and still be silent.

Only audio-bearing inputs are listed. Image, colour and text sources are
filtered out; metering them would fill the panel with permanent silence.

## When the dashboard is stale

If OBS stops answering, the numbers freeze at their last values. The connection
chip changes to `OFFLINE` (or `RETRY #n` while reconnecting) and the status bar
carries the reason and a hint. Reconnection is automatic, with backoff up to 15
seconds.

Pausing with `space` freezes the metrics but deliberately **not** the connection
state, so a paused dashboard can never hide the fact that OBS went away.
