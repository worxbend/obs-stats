# Themes

`obs-stats` ships a signature `grafana` dark theme alongside the existing palettes. Press `F2` or `t` to open the picker; moving the
cursor restyles the entire UI live, `Enter` keeps it, `Esc` puts the old one
back.

<img src="images/themes.svg" alt="The theme picker showing palette swatches and a live full-UI preview" width="900">

The default is deep graphite with orange branding, cyan throughput accents, violet recording signals, and distinct green/amber/red severity colors. Existing configured themes remain unchanged.

## The list

| id | Name | | id | Name |
|---|---|---|---|---|
| `grafana` | Grafana (default) | | | |
| `claude` | Claude | | `kanagawa` | Kanagawa |
| `obs` | OBS Studio | | `rose-pine` | Rosé Pine |
| `btop` | Btop | | `one-dark` | One Dark |
| `nord` | Nord | | `monokai` | Monokai |
| `dracula` | Dracula | | `ayu-dark` | Ayu Dark |
| `gruvbox` | Gruvbox Dark | | `github-dark` | GitHub Dark |
| `gruvbox-light` | Gruvbox Light | | `github-light` | GitHub Light |
| `solarized-dark` | Solarized Dark | | `synthwave` | Synthwave |
| `solarized-light` | Solarized Light | | `matrix` | Matrix |
| `tokyo-night` | Tokyo Night | | `high-contrast` | High Contrast |
| `catppuccin-mocha` | Catppuccin Mocha | | `mono` | Mono (TTY-safe) |
| `catppuccin-latte` | Catppuccin Latte | | `everforest` | Everforest |

`obs-stats themes` prints the same list and marks the active one.

Four are light: `gruvbox-light`, `solarized-light`, `catppuccin-latte`,
`github-light`.

## Choosing one

Persistently, in `~/.config/obs-stats/config.yml`:

```yaml
ui:
  theme: tokyo-night
```

For one run:

```sh
obs-stats --theme nord
```

Lookup is forgiving about case and separators — `Tokyo_Night`, `tokyo-night`
and `TOKYO NIGHT` all resolve to the same palette. An unknown id on the command
line is an error listing how to find the valid ones; an unknown id in the config
file is reported at startup with the offending key.

## Accessibility

**`high-contrast`** uses pure black, white, and fully saturated primaries. Every
state is distinguished by more than colour alone: focused panels get an explicit `>` title marker
as well as an accent border, and the health verdict is a word before it is a
colour.

**`mono`** uses only the 16 ANSI colours and never paints a background of its
own, so it inherits whatever your terminal already uses. That makes it the right
choice for a Linux virtual console, a serial console, or any terminal without
truecolour support.

Pair either with `--ascii` on a terminal whose font lacks the box-drawing and
block ranges:

```sh
obs-stats --theme mono --ascii
```

## How a theme is built

A theme is a flat palette of **semantic roles**. No widget ever names a literal
colour — it asks for `theme.danger` or `theme.border_focus`. That is what lets
the picker restyle everything in a single frame, and it is why adding a palette
never requires touching a widget.

| Role | Used for |
|---|---|
| `bg` | Painted behind the whole UI. `Color::Reset` inherits the terminal's own. |
| `fg` | Body text. |
| `muted` | Labels, units, inactive rows. |
| `accent` | App name, selected tab, primary gauges. |
| `accent_alt` | Secondary series and highlights. |
| `border` | Unfocused panel borders, empty bar segments. |
| `border_focus` | The focused panel's border. |
| `success` / `warning` / `danger` | The health scale, shared by every gauge. |
| `info` | Recording state, preview scene, informational rows. |
| `highlight_bg` / `highlight_fg` | The selected row. |

Two helpers keep colouring consistent across panels: `color_when_high` for values that
get worse as they grow (CPU, congestion, drop rates) and `color_when_low` for values
that get worse as they shrink (FPS against target, free disk). So an amber
number means the same thing wherever you see it.

## Adding a theme

Palettes are `const` data in [`src/tui/theme.rs`](../src/tui/theme.rs). Append a
`Theme` to the `THEMES` array; the picker, the `themes` subcommand, config
validation and the `--theme` flag all read from that one array, so nothing else
needs changing.

The test suite then checks the new entry automatically: ids are unique and
kebab-case, every theme resolves by its own id, selected rows stay readable
(`highlight_bg != highlight_fg`), and every palette renders every view without
panicking.
