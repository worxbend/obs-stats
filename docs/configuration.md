# Configuration

`obs-stats` works with no configuration at all: with no file present it talks to
a stock OBS on `127.0.0.1:4455` and reads the password from
`$OBS_WEBSOCKET_PASSWORD`.

Settings resolve in this order, each layer overriding the one before it:

1. Built-in defaults
2. The config file
3. Command-line flags

The password is never part of that chain — see [Secrets](#secrets).

## Where the file lives

| Platform | Path |
|---|---|
| Linux | `~/.config/obs-stats/config.yml` |
| macOS | `~/Library/Application Support/obs-stats/config.yml` |
| Windows | `%APPDATA%\obs-stats\config\config.yml` |

`$OBS_STATS_CONFIG` overrides this completely — it names a **file**, not a
directory. `--config <path>` overrides that in turn, for one run.

Write a commented starter file with:

```sh
obs-stats init
```

Check what is actually in effect — file, flags and defaults combined — with:

```sh
obs-stats config
```

That also reports whether the password variable is set, without printing it.

## Reference

### `connection`

| Key | Type | Default | Notes |
|---|---|---|---|
| `host` | string | `127.0.0.1` | A bare host or IP, not a URL. IPv6 literals may be written bare (`::1`) or bracketed. |
| `port` | 1–65535 | `4455` | From OBS → Tools → WebSocket Server Settings. |
| `password_env` | string | `OBS_WEBSOCKET_PASSWORD` | The **name** of an environment variable. Set to `""` for an OBS with authentication disabled. |
| `connect_timeout_ms` | > 0 | `3000` | Applies to the TCP connect and the handshake together. |

### `ui`

| Key | Type | Default | Notes |
|---|---|---|---|
| `theme` | theme id | `claude` | See [themes.md](themes.md), or run `obs-stats themes`. |
| `poll_interval_ms` | 100–60000 | `1000` | How often the hot metrics are polled. Scenes, inputs, profiles and collections refresh at a tenth of this rate. |
| `history_seconds` | 10–3600 | `120` | How much history the charts retain. Independent of the poll rate — changing the rate changes the sample count, not the window. |
| `unicode` | bool | `true` | Block glyphs, braille charts, rounded borders. Turn off for a Linux VT. |
| `show_icons` | bool | `true` | The glyph column in tab titles. |
| `mouse` | bool | `true` | Scroll to move a selection, click a tab to switch. |
| `start_view` | view id | `overview` | One of `overview`, `performance`, `stream`, `scenes`, `audio`, `log`. |

Unknown keys are an error rather than being silently ignored, so a typo like
`thmee:` is reported at startup instead of quietly doing nothing.

### `alerts`

Frame-loss detection and desktop notifications. See [alerts.md](alerts.md) for
what triggers one and how to tune it.

| Key | Type | Default | Notes |
|---|---|---|---|
| `enabled` | bool | `true` | Detect frame loss at all. Turning this off also silences notifications. |
| `desktop` | bool | `true` | Send a desktop notification as well as the in-app banner. Linux only; skipped when there is no session bus. |
| `min_frames` | u64 | `3` | Frames that must be lost within the window before alerting. `0` is treated as `1`. |
| `window_seconds` | 1–600 | `10` | How far back to count losses. |
| `cooldown_seconds` | 0–3600 | `60` | Minimum gap between two alerts about the same cause. |

## Command-line flags

Every flag is global — it works with or without a subcommand.

| Flag | Overrides |
|---|---|
| `--config <path>` | The config file location |
| `--host <host>` | `connection.host` |
| `--port <port>` | `connection.port` |
| `--password-env <name>` | `connection.password_env` |
| `--theme <id>` | `ui.theme` |
| `--interval <ms>` | `ui.poll_interval_ms` |
| `--view <id>` | `ui.start_view` |
| `--ascii` | Sets `ui.unicode: false` and `ui.show_icons: false` |
| `--no-mouse` | Sets `ui.mouse: false` |
| `--no-alerts` | Sets `alerts.enabled: false` |
| `--no-notify` | Sets `alerts.desktop: false` |
| `--log-file <path>` | Enables debug logging to that file |

A bad `--theme` or `--view` fails at startup with the list of valid values,
rather than silently falling back to the default.

## Subcommands

| Command | Effect |
|---|---|
| *(none)* | Launch the dashboard |
| `init` | Write a commented starter config. Refuses to overwrite an existing file. |
| `themes` | List the built-in themes, marking the active one |
| `config` | Print the effective configuration as YAML |
| `once` | Connect, print one snapshot as JSON, exit |

## Secrets

The OBS password is read from an environment variable whose *name* you
configure. The password itself is never a config value.

```sh
export OBS_WEBSOCKET_PASSWORD='...'
obs-stats
```

This is deliberate, and it buys three things:

- The config file is always safe to commit and to paste into a bug report.
- Passwords cannot end up in a dotfiles repository by accident.
- Every code path that could print a password holds a `Redacted` wrapper whose
  `Debug` and `Display` both render `<redacted>`, so a stray `{:?}` cannot leak
  it.

Two failure modes are reported explicitly rather than surfacing later as a
confusing authentication error:

- A value containing control characters — almost always a trailing newline from
  `export PW=$(cat secret)`. Use `$(cat secret | tr -d '\n')`.
- A value over 512 bytes.

If OBS requires authentication and no password is available, `obs-stats` says so
directly instead of letting the server close the connection with an opaque code.

## Runtime adjustments

Some settings can be changed while running. They apply immediately and are
reflected in the status bar, but are **not** written back to disk — the config
file is yours to edit.

| Key | Changes |
|---|---|
| `+` / `-` | Poll interval, halving or doubling within the 100–60000 ms bounds |
| `t` / `F2` → `Enter` | Theme |
| `u` | Unicode drawing |

Changing the poll interval also resizes the history buffers, so
`history_seconds` keeps covering the same span of time.

## Logging

Logging is off by default: a TUI cannot share stdout with a logger, and a
dashboard nobody asked to debug should not be writing files.

```sh
obs-stats --log-file ~/obs-stats.log
OBS_STATS_LOG=obs_stats=trace obs-stats --log-file ~/obs-stats.log
```

`OBS_STATS_LOG` takes a `tracing` filter directive; the default is
`obs_stats=debug`. The log never contains the password.
