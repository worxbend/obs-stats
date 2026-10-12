"""Stdlib-only tests for the real-terminal smoke harness.

Run: python3 -m unittest discover -s tests -p 'test_pty_smoke.py' -v
"""
import importlib.util
import os
from pathlib import Path
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "pty_smoke.py"


def load_harness():
    spec = importlib.util.spec_from_file_location("pty_smoke", SCRIPT)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class ScreenTests(unittest.TestCase):
    def test_incremental_redraw_replaces_old_view(self):
        self.assertTrue(SCRIPT.is_file(), "PTY harness has not been implemented")
        screen = load_harness().Screen(40, 4)
        screen.feed(b"\x1b[2J\x1b[2;1H\x1b[38;2;1;2;3mTELEMETRY / Overview")
        self.assertIn("TELEMETRY / Overview", screen.text())
        screen.feed(b"\x1b[2;")
        screen.feed(b"13HPerformance\x1b[K")
        self.assertIn("TELEMETRY / Performance", screen.text())
        self.assertNotIn("Overview", screen.text())


    def test_split_utf8_osc_and_colour_do_not_pollute_labels(self):
        screen = load_harness().Screen(30, 3)
        output = b"\x1b]0;hidden title\x1b\\\x1b[?1049h\x1b[1;1H\x1b[31mOFFLINE\x1b[0m " + "—".encode()
        for byte in output:
            screen.feed(bytes([byte]))
        self.assertTrue(screen.text().startswith("OFFLINE —"))
        self.assertNotIn("hidden title", screen.text())

    def test_erasure_and_resize_remove_historical_screen(self):
        screen = load_harness().Screen(30, 3)
        screen.feed(b"old view\x1b[2;1HPAUSED\x1b[1;1H\x1b[2Jcurrent")
        self.assertNotIn("PAUSED", screen.text())
        self.assertNotIn("old view", screen.text())
        self.assertIn("current", screen.text())
        screen.resize(18, 6)
        self.assertNotIn("current", screen.text())
        screen.feed(b"\x1b[1;1Hterminal too small")
        self.assertEqual(screen.text().splitlines()[0], "terminal too small")
        self.assertEqual(len(screen.text().splitlines()), 6)


@unittest.skipUnless(os.name == "posix", "real PTYs require POSIX")
class HarnessTests(unittest.TestCase):
    def test_zero_exit_without_a_dashboard_is_not_success(self):
        import json
        import subprocess
        import sys
        import tempfile

        with tempfile.TemporaryDirectory(prefix="obs-stats-smoke-test-") as directory:
            result = subprocess.run(
                [sys.executable, str(SCRIPT), "--binary", "/bin/true",
                 "--timeout", "2", "--output-dir", directory],
                capture_output=True, text=True, timeout=5,
            )
            self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
            report = json.loads((Path(directory) / "result.json").read_text())
            self.assertFalse(report["ok"])
            self.assertIn("startup", report["error"])


    def test_stuck_dashboard_times_out_and_child_is_reaped(self):
        import json
        import subprocess
        import sys
        import tempfile

        with tempfile.TemporaryDirectory(prefix="obs-stats-smoke-test-") as directory:
            root = Path(directory)
            child = root / "stuck-dashboard"
            pid_file = root / "pid"
            child.write_text(
                "#!" + sys.executable + "\n"
                "import os, signal, time, tty\n"
                f"open({str(pid_file)!r}, 'w').write(str(os.getpid()))\n"
                "tty.setraw(0)\n"
                "signal.signal(signal.SIGTERM, signal.SIG_IGN)\n"
                "os.write(1, b'\\x1b[?1049hobs-stats OFFLINE no samples, no pretend numbers')\n"
                "time.sleep(60)\n",
                encoding="utf-8",
            )
            child.chmod(0o700)
            result = subprocess.run(
                [sys.executable, str(SCRIPT), "--binary", str(child),
                 "--timeout", "0.5", "--stage-timeout", "0.2", "--output-dir", directory],
                capture_output=True, text=True, timeout=5,
            )
            self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
            report = json.loads((root / "result.json").read_text())
            self.assertFalse(report["ok"])
            self.assertIn("pre-overview: timed out", report["error"])
            self.assertEqual(report["stages"][0]["stage"], "startup")
            with self.assertRaises(ProcessLookupError):
                os.kill(int(pid_file.read_text()), 0)


if __name__ == "__main__":
    unittest.main()
