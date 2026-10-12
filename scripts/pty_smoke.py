#!/usr/bin/env python3
"""Real-PTY acceptance check; stdlib only, no OBS or personal configuration.

Linux/POSIX usage:
    cargo build --locked --bin obs-stats
    python3 scripts/pty_smoke.py --binary target/debug/obs-stats

Optional --output-dir saves session.ansi, per-stage screens and result.json on
success or failure. Temporary configuration respects Python's TMPDIR handling.
The small ANSI screen model consumes incremental Crossterm redraws, not a grep
of historical output. --ascii keeps every rendered cell single-width.
"""
import codecs


class Screen:
    """Minimal incremental VT screen for the ASCII Crossterm backend.

    Tracks cursor addressing/movement, erasure and printable text; ignores
    colour, mouse and cursor-visibility modes. Not a general terminal emulator.
    """

    def __init__(self, width, height):
        self.decoder = codecs.getincrementaldecoder("utf-8")("strict")
        self.pending = ""
        self.resize(width, height)

    def resize(self, width, height):
        self.width, self.height = width, height
        self.cells = [[" "] * width for _ in range(height)]
        self.x = self.y = 0

    def text(self):
        return "\n".join("".join(row) for row in self.cells)

    def feed(self, data):
        text = self.pending + self.decoder.decode(data)
        self.pending = ""
        i = 0
        while i < len(text):
            char = text[i]
            if char == "\x1b":
                start = i
                if i + 1 == len(text):
                    self.pending = text[start:]
                    break
                if text[i + 1] == "[":
                    i += 2
                    end = i
                    while end < len(text) and not ("@" <= text[end] <= "~"):
                        end += 1
                    if end == len(text):
                        self.pending = text[start:]
                        break
                    self._csi(text[i:end], text[end])
                    i = end + 1
                    continue
                if text[i + 1] == "]":
                    # OSC title/hyperlink, terminated by BEL or ST.
                    end = i + 2
                    while end < len(text) and text[end] != "\x07" and text[end:end + 2] != "\x1b\\":
                        end += 1
                    if end == len(text) or (text[end] == "\x1b" and end + 1 == len(text)):
                        self.pending = text[start:]
                        break
                    i = end + (1 if text[end] == "\x07" else 2)
                    continue
                i += 2
                continue
            if char == "\r":
                self.x = 0
            elif char == "\n":
                self.y = min(self.height - 1, self.y + 1)
            elif char == "\b":
                self.x = max(0, self.x - 1)
            elif char >= " " and char != "\x7f":
                if self.x < self.width:
                    self.cells[self.y][self.x] = char
                self.x += 1
            i += 1

    def _csi(self, args, final):
        if args.startswith("?") or final == "m":
            return
        values = [int(part or "0") for part in args.split(";")] if args else [0]
        n = values[0] or 1
        if final in ("H", "f"):
            self.y = max(0, min(self.height - 1, n - 1))
            self.x = max(0, min(self.width - 1, (values[1] or 1) - 1 if len(values) > 1 else 0))
        elif final == "G":
            self.x = min(self.width - 1, n - 1)
        elif final == "A":
            self.y = max(0, self.y - n)
        elif final == "B":
            self.y = min(self.height - 1, self.y + n)
        elif final == "C":
            self.x = min(self.width - 1, self.x + n)
        elif final == "D":
            self.x = max(0, self.x - n)
        elif final == "J":
            cursor = self.y * self.width + self.x
            for y, row in enumerate(self.cells):
                for x in range(self.width):
                    offset = y * self.width + x
                    if values[0] in (2, 3) or (values[0] == 0 and offset >= cursor) or (values[0] == 1 and offset <= cursor):
                        row[x] = " "
        elif final == "K":
            for x in range(self.width):
                if values[0] == 2 or (values[0] == 0 and x >= self.x) or (values[0] == 1 and x <= self.x):
                    self.cells[self.y][x] = " "


def run_smoke(binary, timeout=30.0, stage_timeout=3.0, output_dir=None):
    import errno
    import fcntl
    import json
    import os
    from pathlib import Path
    import pty
    import select
    import signal
    import socket
    import struct
    import subprocess
    import tempfile
    import termios
    import time

    report = {"ok": False, "stages": [], "binary": str(binary)}
    transcript = bytearray()
    screen = Screen(132, 38)
    master = slave = None
    proc = None
    before = None
    deadline = time.monotonic() + timeout
    artifacts = Path(output_dir) if output_dir else None
    if artifacts:
        artifacts.mkdir(parents=True, exist_ok=True)

    def set_size(width, height):
        assert slave is not None
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", height, width, 0, 0))
        screen.resize(width, height)

    def child_terminal():
        os.setsid()
        fcntl.ioctl(0, termios.TIOCSCTTY, 0)

    def read_output(wait):
        assert master is not None
        ready, _, _ = select.select([master], [], [], max(0, wait))
        if not ready:
            return False
        try:
            data = os.read(master, 262144)
        except OSError as error:
            if error.errno == errno.EIO:
                return False
            raise
        transcript.extend(data)
        screen.feed(data)
        return bool(data)

    def stage(name, present=(), absent=(), key=None, predicate=None):
        assert master is not None and proc is not None
        if key:
            os.write(master, key)
        start = len(transcript)
        end = min(deadline, time.monotonic() + stage_timeout)
        while time.monotonic() < end:
            read_output(min(0.05, end - time.monotonic()))
            text = screen.text()
            if len(transcript) > start and all(label in text for label in present) and all(label not in text for label in absent) and (predicate is None or predicate(text)):
                report["stages"].append({"stage": name, "bytes": len(transcript) - start,
                                         "size": [screen.width, screen.height]})
                if artifacts:
                    (artifacts / (name + ".txt")).write_text(text, encoding="utf-8")
                return
            if proc.poll() is not None:
                raise RuntimeError(f"{name}: process exited {proc.returncode} before expected screen {present!r}")
        raise RuntimeError(f"{name}: timed out waiting for screen {present!r}, absent {absent!r}")

    try:
        # Keep an allocated TCP port bound but NOT listening. No OBS can answer
        # and no other process can claim the port between allocation and launch.
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as closed_port, tempfile.TemporaryDirectory(prefix="obs-stats-pty-") as scratch:
            closed_port.bind(("127.0.0.1", 0))
            port = closed_port.getsockname()[1]
            config = Path(scratch) / "nonexistent-config.yml"
            master, slave = pty.openpty()
            before = termios.tcgetattr(slave)
            set_size(132, 38)
            env = dict(os.environ, TERM="xterm-256color")
            env.pop("OBS_STATS_CONFIG", None)
            args = [str(Path(binary).resolve()), "--host", "127.0.0.1", "--port", str(port),
                    "--password-env", "", "--config", str(config), "--no-notify",
                    "--ascii", "--theme", "grafana", "--view", "overview"]
            proc = subprocess.Popen(args, stdin=slave, stdout=slave, stderr=slave,
                                    env=env, preexec_fn=child_terminal, close_fds=True)
            stage("startup", ("obs-stats", "OFFLINE", "no samples, no pretend numbers"),
                  absent=("WARMING UP", "CONNECTED", "1920x1080"))
            raw = termios.tcgetattr(slave)
            if raw[3] & (termios.ICANON | termios.ECHO):
                raise RuntimeError("startup: terminal did not enter raw mode")
            report["raw_mode_entered"] = True

            views = [("Overview", "Overview / telemetry link"),
                     ("Performance", "Awaiting first OBS poll / no measurements"),
                     ("Stream", "Awaiting output status"),
                     ("Scenes", "no scenes reported yet"),
                     ("Audio", "Audio inputs"), ("Log", "Events")]
            for number, (view, body) in enumerate(views, 1):
                if number == 1:
                    # Startup already validated Overview; first leave it so the
                    # 1 binding must produce a fresh visible state transition.
                    stage("pre-overview", ("TELEMETRY / Log", "Events"), key=b"6")
                stage("view-" + view.lower(), ("TELEMETRY / " + view, body), key=str(number).encode())

            stage("help-open", ("Help", "ESC close", "Views"), key=b"?")
            stage("help-close", ("TELEMETRY / Log",), ("ESC close",), key=b"\x1b")
            stage("themes-open", ("Appearance", "PREVIEW", "Esc reverts"), key=b"t")
            stage("themes-preview", ("Appearance",), key=b"j",
                  predicate=lambda text: "Grafana" not in text.splitlines()[-1])
            stage("themes-close", ("Grafana",), ("Appearance",), key=b"\x1b")
            stage("leader", ("space", "+Find", "+Views"), key=b" ")
            stage("palette-open", ("COMMANDS", "matches"), key=b"ff")
            stage("palette-filter", ("> view performance", "1 matches"), key=b"view performance")
            stage("palette-execute", ("TELEMETRY / Performance", "Awaiting first OBS poll"),
                  ("COMMANDS",), key=b"\r")
            stage("command-input", (":view stream",), key=b":view stream")
            stage("command-execute", ("TELEMETRY / Stream", "Awaiting output status"),
                  (":view stream",), key=b"\r")
            stage("pause", ("PAUSED",), key=b"p")
            stage("resume", ("Grafana",), ("PAUSED",), key=b"p")

            # SGR left click on the tab text, found from the current rendered
            # frame instead of hard-coded geometry. Coordinates are 1-based.
            rows = screen.text().splitlines()
            row = next((i for i, line in enumerate(rows) if "2 Performance" in line), None)
            if row is None:
                raise RuntimeError("mouse-tab: rendered Performance tab is missing")
            column = rows[row].index("2 Performance") + 2
            mouse = f"\x1b[<0;{column};{row + 1}M\x1b[<0;{column};{row + 1}m".encode()
            stage("mouse-tab", ("TELEMETRY / Performance", "Awaiting first OBS poll"), key=mouse)
            stage("overview-return", ("TELEMETRY / Overview", "no samples, no pretend numbers"), key=b"1")

            for width, height in [(80, 24), (96, 28), (160, 50), (40, 12), (18, 6), (132, 38)]:
                set_size(width, height)
                os.kill(proc.pid, signal.SIGWINCH)
                if width < 20 or height < 8:
                    stage(f"resize-{width}x{height}", ("terminal too small",))
                else:
                    stage(f"resize-{width}x{height}", ("TELEMETRY / Overview",),
                          predicate=lambda text: "Grafana" in text.splitlines()[-1])

            os.write(master, b"q")
            while proc.poll() is None and time.monotonic() < deadline:
                read_output(min(0.05, deadline - time.monotonic()))
            if proc.poll() is None:
                raise RuntimeError("quit: process did not exit before overall timeout")
            # Drain teardown escapes after wait; retaining the slave lets us
            # inspect its termios flags even after the child closed its fd.
            while read_output(0):
                pass
            report["exit_code"] = proc.returncode
            report["terminal_restored"] = before == termios.tcgetattr(slave)
            report["alternate_screen_entered"] = b"\x1b[?1049h" in transcript
            report["alternate_screen_left"] = b"\x1b[?1049l" in transcript
            report["mouse_capture_disabled"] = b"\x1b[?1006l" in transcript
            if proc.returncode != 0:
                raise RuntimeError(f"quit: exit code {proc.returncode}")
            for check in ("terminal_restored", "alternate_screen_entered", "alternate_screen_left", "mouse_capture_disabled"):
                if not report[check]:
                    raise RuntimeError("quit: " + check + " failed")
            if b"panic" in transcript.lower():
                raise RuntimeError("panic text in terminal transcript")
            if config.exists():
                raise RuntimeError("dashboard unexpectedly wrote its scratch configuration")
            report["ok"] = True
    except (OSError, RuntimeError, ValueError) as error:
        report["error"] = str(error)
    finally:
        if proc is not None and proc.poll() is None:
            # Bound cleanup independently; kill the session group, not just the
            # leader, so a failed test cannot leak helpers into the CI runner.
            try:
                os.killpg(proc.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass  # The child exited between poll() and killpg().
            try:
                proc.wait(timeout=1)
            except subprocess.TimeoutExpired:
                try:
                    os.killpg(proc.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                proc.wait(timeout=1)
        if slave is not None:
            report.setdefault("terminal_restored", before == termios.tcgetattr(slave))
            os.close(slave)
        if master is not None:
            os.close(master)
        report["captured_bytes"] = len(transcript)
        if artifacts:
            (artifacts / "session.ansi").write_bytes(transcript)
            (artifacts / "last-screen.txt").write_text(screen.text(), encoding="utf-8")
            (artifacts / "result.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    return report


def main():
    import argparse
    import json
    import math
    import os
    import sys

    def positive_seconds(value):
        seconds = float(value)
        if not math.isfinite(seconds) or seconds <= 0:
            raise argparse.ArgumentTypeError("must be a finite positive number of seconds")
        return seconds

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/debug/obs-stats", help="actual compiled obs-stats executable")
    parser.add_argument("--timeout", type=positive_seconds, default=30.0, help="overall child-process deadline, seconds (default: 30)")
    parser.add_argument("--stage-timeout", type=positive_seconds, default=3.0, help="per-screen deadline, seconds (default: 3)")
    parser.add_argument("--output-dir", help="optional diagnostic artifact directory")
    args = parser.parse_args()
    if os.name != "posix":
        parser.error("real PTY smoke coverage requires a POSIX host")
    report = run_smoke(args.binary, args.timeout, args.stage_timeout, args.output_dir)
    print(json.dumps(report, indent=2))
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
