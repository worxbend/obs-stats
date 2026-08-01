# Troubleshooting

## It says `OFFLINE` and keeps retrying

The status bar carries the reason and a hint. The usual causes, in order of
likelihood:

**The WebSocket server is off.** In OBS: *Tools → WebSocket Server Settings →
Enable WebSocket server*. It is off by default in a fresh install.

**The port is different.** The same dialog shows it. Default is 4455.

```sh
obs-stats --port 4455
```

**OBS is on another machine.** By default obs-websocket binds all interfaces,
but the firewall on the OBS host still has to allow the port.

```sh
obs-stats --host 192.168.1.20
```

**OBS is in a sandbox.** Flatpak and Snap builds of OBS may not expose the port
to the host. `obs-stats once` will confirm quickly either way. (If OBS reports a
platform of `org.freedesktop.platform`, it is a Flatpak.)

Confirm the socket is reachable at all:

```sh
curl -sv --http1.1 http://127.0.0.1:4455 2>&1 | head -5
```

A `400 Bad Request` from the server means the port is open and something is
listening — that is a good sign, not a failure.

## It says `FAILED: authentication failed`

The password does not match. In OBS: *Tools → WebSocket Server Settings → Show
Connect Info*.

```sh
export OBS_WEBSOCKET_PASSWORD='...'
obs-stats
```

`FAILED` is terminal by design — a wrong password will be wrong on every retry,
so `obs-stats` stops rather than hammering the server.

### `$… contains control characters (a trailing newline?)`

Almost always this:

```sh
export OBS_WEBSOCKET_PASSWORD=$(cat secret)      # keeps the newline
export OBS_WEBSOCKET_PASSWORD=$(cat secret | tr -d '\n')   # correct
```

`obs-stats` fails loudly here rather than sending the newline and letting you
puzzle over an authentication error.

### `OBS WebSocket requires a password but none was configured`

The variable is unset or empty. Check which variable is being read:

```sh
obs-stats config | head -5
```

It reports whether the password is available, without printing it.

If OBS has authentication *disabled*, this error should not appear — but if you
have set `password_env: ""`, `obs-stats` connects anonymously by design.

## The dashboard is running but the numbers do not change

Check the poll counter beside the connection chip (`● CONNECTED #1234`). If it
is advancing, data is flowing and the numbers really are steady.

If it is frozen, you are paused. Look for the `PAUSED` badge on the right of the
status bar and press `space`.

## Everything is boxes, question marks, or mojibake

Your terminal font lacks the box-drawing, block or braille ranges. ASCII mode
substitutes all of them, charts included.

```sh
obs-stats --ascii
```

Or press `u` while running. For a Linux virtual console, combine it with the
TTY-safe palette:

```sh
obs-stats --theme mono --ascii
```

## Colours look wrong or washed out

Most themes use 24-bit colour. If your terminal only supports 256 colours the
palette will be approximated.

```sh
echo $COLORTERM      # should print truecolor or 24bit
```

`tmux` often strips truecolour. Add to `~/.tmux.conf`:

```tmux
set -g default-terminal "tmux-256color"
set -ga terminal-overrides ",*256col*:Tc"
```

Or use `--theme mono`, which only needs the 16 ANSI colours.

## I cannot select text with the mouse

Mouse capture takes precedence over your terminal's native selection.

```sh
obs-stats --no-mouse
```

Or set `ui.mouse: false`. Most terminals also let you hold `Shift` to bypass
capture for one selection.

## The audio panel is empty

`obs-stats` lists only audio-bearing inputs — image, colour and text sources are
filtered out, since metering them would fill the panel with permanent silence.

If a source you expect *is* audio-bearing and still does not appear, it may use
a plugin input kind that is not recognised. Check what OBS reports:

```sh
obs-stats once | jq '.inputs'
```

Please open an issue with that output and the missing source's kind.

## Meters stay silent even though audio is playing

Volume meters arrive as high-volume events, not polls, so a very slow or
saturated connection can starve them. Check that the poll counter is advancing
first.

Note that a **muted** input always shows an empty meter, whatever its level —
that is deliberate, so mute state is never ambiguous.

## I am not getting desktop notifications

Check what `obs-stats` sees:

```sh
cargo run --example notification_check
```

It reports whether a session bus was found, whether the notifier started, and
sends one notification of each severity.

**"No session bus was found"** — there is no desktop to notify. Normal on a bare
TTY, over ssh, or inside a container. Alerts still appear in the dashboard.

**The notifier starts but nothing appears** — your notification daemon is not
running, or do-not-disturb is on. Confirm a daemon is listening:

```sh
gdbus call --session --dest org.freedesktop.Notifications \
  --object-path /org/freedesktop/Notifications \
  --method org.freedesktop.Notifications.GetServerInformation
```

That should name your daemon. If it errors, nothing on your desktop is
handling notifications.

**They stopped part-way through a session** — after three consecutive delivery
failures `obs-stats` gives up for the session rather than retrying forever. Run
with `--log-file` to see the failures.

Notifications are Linux-only. On macOS and Windows alerts appear in the
dashboard only.

## I am getting too many alerts

Raise the threshold rather than turning them off — a machine that always skips a
few render frames is better served by a higher bar:

```yaml
alerts:
  min_frames: 10
  cooldown_seconds: 300
```

Or narrow them: `--no-notify` keeps the banner and stops the desktop popups.
See [alerts.md](alerts.md) for the full set of knobs.

## Alerts never fire even though frames are being dropped

Alerts are about frames lost *recently*, not the totals in the panels. A
counter reading 400 dropped frames from an hour ago will not alert; only new
losses do.

They also require an output to be running — the renderer can skip while OBS
sits idle, and nobody is watching then.

To confirm the machinery works, set `min_frames: 1` and `cooldown_seconds: 0`
temporarily.

## The terminal is left broken after a crash

It should not be: `obs-stats` installs a panic hook that restores the terminal
before the backtrace prints. If it ever happens anyway:

```sh
reset
```

and please open an issue with the backtrace.

## Getting a useful bug report together

```sh
obs-stats config                       # effective settings; no secrets
obs-stats once > snapshot.json         # the numbers, as JSON
obs-stats --log-file /tmp/obs-stats.log
```

None of these contain the password. The config dump reports only whether the
variable is set.

Please include your OBS version, obs-websocket version (both appear in the
`obs` section of `once`), terminal emulator, and `$TERM`/`$COLORTERM`.
