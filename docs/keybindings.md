# Key bindings

Press `?` or `F1` in the app for the same reference, generated from the keymap
itself — the two cannot drift apart.

## Navigation

| Key | Action |
|---|---|
| `Tab` | Next view |
| `Shift-Tab` | Previous view |
| `→` / `l` | Next view |
| `←` / `h` | Previous view |
| `1` … `6` | Jump straight to a view |
| `↓` / `j` | Move the list cursor down |
| `↑` / `k` | Move the list cursor up |
| `PgDn` / `PgUp` | Move the cursor ten rows |
| `Home` / `g` | Jump to the top |
| `End` / `G` | Jump to the bottom |

The cursor clamps rather than wrapping: in a 500-line log, jumping from the
newest entry to the oldest on one keypress is disorienting.

Each view keeps its own cursor, so switching away and back does not lose your
place. If OBS deletes the scene you had selected, the cursor is pulled back into
range rather than pointing past the end.

Cursor keys only do something on views with a list — Scenes, Audio and Log.

## Views

| Key | View | Shows |
|---|---|---|
| `1` | Overview | Health verdict, CPU/memory/FPS gauges, live output cards |
| `2` | Performance | Frame pacing: FPS vs target, render budget, skip rates |
| `3` | Stream | Stream and recording detail with a live bitrate chart |
| `4` | Scenes | Scene list with program/preview, profiles and collections |
| `5` | Audio | Audio inputs with faders and live peak meters |
| `6` | Log | Events reported by OBS since this session started |

## Data

| Key | Action |
|---|---|
| `r` | Refresh now |
| `space` / `p` | Pause or resume the display |
| `+` / `=` | Poll faster (halve the interval) |
| `-` / `_` | Poll slower (double the interval) |

Pausing freezes the metrics but **not** the connection state — a paused
dashboard still shows you when OBS goes away.

Refreshing while paused resumes first. A refresh you cannot see would not be
what you meant.

The poll interval is clamped to 100–60000 ms. At either limit the key reports
that it is already there rather than silently doing nothing. Changing the rate
also resizes the history buffers, so the charts keep covering the same span of
time.

## Appearance

| Key | Action |
|---|---|
| `t` / `F2` | Open the theme picker |
| `u` | Toggle Unicode drawing |

In the picker, moving the cursor previews the theme across the **whole** UI, not
just a swatch. `Enter` keeps it; `Esc` puts the old one back. Neither writes to
the config file — set `ui.theme` for that.

`u` swaps every block glyph, meter segment and rounded border for an ASCII
equivalent. Useful on a Linux virtual console, over a serial link, or in any
terminal whose font lacks the box-drawing range. `--ascii` starts in that mode.

## General

| Key | Action |
|---|---|
| `?` / `F1` | Help overlay |
| `q` / `Esc` | Quit |
| `Ctrl-C` | Quit, from anywhere including overlays |

## Overlays

| Key | Action |
|---|---|
| `Enter` | Apply and close |
| `Esc` | Close, reverting the preview |
| `↑` `↓` `j` `k` `g` `G` `PgUp` `PgDn` | Move the selection |

While an overlay is open the dashboard keys are inert — `Tab` will not switch
views behind the help panel. `q` and `Ctrl-C` still quit.

## Mouse

Enabled unless you pass `--no-mouse` or set `ui.mouse: false`.

| Gesture | Action |
|---|---|
| Scroll wheel | Move the list cursor |
| Left-click a tab | Switch to that view |

Other clicks and drags are ignored, so a stray click cannot change state.

Disable the mouse if you want your terminal's native text selection back —
mouse capture takes precedence over it.
