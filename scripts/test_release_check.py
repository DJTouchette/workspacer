"""Cheap regression tests: no real services, builds, or application processes."""

import importlib.util
import contextlib
import io
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch


SCRIPT = Path(__file__).with_name("release-check.py")
SPEC = importlib.util.spec_from_file_location("release_check", SCRIPT)
runner = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(runner)
child = {"__name__": "test_bootstrap"}
exec(runner.CHILD, child)


class GuardTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.unit = "workspacer-release-check-test.service"
        self.group = "/user.slice/app.slice/" + self.unit
        self.directory = self.root / self.group.lstrip("/")
        self.directory.mkdir(parents=True)
        self.limits = {"memory.high": 1610612736, "memory.max": 2147483648,
                       "memory.swap.max": 0, "memory.oom.group": 1, "pids.max": 128}
        for name, value in {**self.limits, "cpu.max": "100000 100000"}.items():
            (self.directory / name).write_text(str(value))
        self.payload = {"unit": self.unit, "limits": self.limits, "cpu_quota": 100}

    def verify(self, membership=None):
        return child["verify"](self.payload, membership or "0::" + self.group, self.root)

    def test_actual_limits_and_membership(self):
        self.assertEqual(self.verify()["cgroup"], self.group)

    def test_every_missing_or_unenforced_controller_fails_closed(self):
        for name in (*self.limits, "cpu.max"):
            with self.subTest(name=name):
                file = self.directory / name
                original = file.read_text()
                file.unlink()
                with self.assertRaises(OSError):
                    self.verify()
                file.write_text("max" if name != "cpu.max" else "max 100000")
                with self.assertRaises(ValueError):
                    self.verify()
                file.write_text(original)

    def test_inherited_or_wrong_cgroup_refused(self):
        for membership in ("0::/app-workspacer-live.scope", "1:memory:/old",
                           "0::/app-workspacer-live.scope/" + self.unit,
                           "0::/other.service", "0::/../" + self.unit):
            with self.subTest(membership=membership), self.assertRaises(ValueError):
                self.verify(membership)

    def test_invalid_and_unlimited_overrides(self):
        for flags in (["--memory-max", "infinity"], ["--memory-high", "0"],
                      ["--memory-max", "1G"], ["--cpu-quota", "0"],
                      ["--tasks-max", "-1"], ["--timeout", "0"]):
            with self.subTest(flags=flags), self.assertRaises(SystemExit), contextlib.redirect_stderr(io.StringIO()):
                runner.parse_args([*flags, "--", "true"])

    def test_environment_identity_and_explicit_secrets(self):
        with patch.dict(os.environ, {"HOME": "/caller home", "PATH": "/caller/bin",
                                     "SECRET": "literal $value %s", "GOMEMLIMIT": "9GiB"}, clear=True):
            env = runner.environment([])
            self.assertEqual(env["HOME"], "/caller home")
            self.assertEqual(env["PATH"], "/caller/bin")
            self.assertNotIn("SECRET", env)
            self.assertEqual(env["GOMEMLIMIT"], "1GiB")
            env = runner.environment(["SECRET", "HOME=/scratch", "GOMEMLIMIT=256MiB"])
            self.assertEqual(env["SECRET"], "literal $value %s")
            self.assertEqual(env["HOME"], "/scratch")
            self.assertEqual(env["GOMEMLIMIT"], "256MiB")
            with self.assertRaises(ValueError):
                runner.environment(["UNSET"])


FAKE_RUN = r'''#!/usr/bin/env python3
import json, os, pathlib, sys, time
root = pathlib.Path(os.environ["FAKE_ROOT"])
payload = json.load(sys.stdin)
(root / "request.json").write_text(json.dumps({"args": sys.argv[1:], "payload": payload}))
print("fake service output", flush=True)
if os.environ.get("FAKE_WAIT"):
    # Simulate a start-job race: the first stop arrives before unit creation.
    time.sleep(0.4)
    (root / "active").touch()
    while not (root / "stopped").exists():
        time.sleep(0.02)
sys.exit(int(os.environ.get("FAKE_EXIT", "0")))
'''
FAKE_CTL = r'''#!/usr/bin/env python3
import json, os, pathlib, sys
root = pathlib.Path(os.environ["FAKE_ROOT"])
with (root / "control.jsonl").open("a") as log:
    log.write(json.dumps(sys.argv[1:]) + "\n")
if "show" in sys.argv:
    print("not-found")
elif (root / "active").exists():
    (root / "stopped").touch()
else:
    sys.exit(5)
'''


class LaunchTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="release check $literal %s ")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        for name, content in (("systemd-run", FAKE_RUN), ("systemctl", FAKE_CTL)):
            file = self.root / name
            file.write_text(content)
            file.chmod(0o700)
        self.env = {**os.environ, "PATH": str(self.root) + os.pathsep + os.environ["PATH"],
                    "FAKE_ROOT": str(self.root)}
        self.command = [sys.executable, str(SCRIPT), "--log-dir", str(self.root / "logs"),
                        "--cwd", str(self.root)]

    def launch(self, args=None, **env):
        return subprocess.run([*self.command, *(args or []), "--", "no-such-command"],
                              env={**self.env, **env}, capture_output=True, text=True, timeout=10)

    def request(self):
        return json.loads((self.root / "request.json").read_text())

    def result(self):
        return json.loads(next((self.root / "logs").glob("*/result.json")).read_text())

    def test_properties_and_nonzero_exit_and_logs(self):
        result = self.launch(FAKE_EXIT="37")
        self.assertEqual(result.returncode, 37, result.stderr)
        args = self.request()["args"]
        for property in ("MemoryHigh=1610612736", "MemoryMax=2147483648", "MemorySwapMax=0",
                         "CPUQuota=100%", "TasksMax=128", "OOMPolicy=kill",
                         "KillMode=control-group", "RuntimeMaxSec=3600s", "TimeoutStopSec=5s"):
            self.assertIn("--property=" + property, args)
        for flag in ("--wait", "--pipe", "--collect", "--user", "--slice=app.slice",
                     "--expand-environment=no", "--service-type=exec"):
            self.assertIn(flag, args)
        self.assertNotIn("--scope", args)
        self.assertEqual(self.result()["exitCode"], 37)
        self.assertIn("fake service output", next((self.root / "logs").glob("*/output.log")).read_text())

    def test_success(self):
        self.assertEqual(self.launch().returncode, 0)
        self.assertFalse((self.root / "control.jsonl").exists())

    def test_cwd_argv_and_environment_are_data(self):
        argv = ["./program with spaces", "", "$(touch OWNED)", "`touch OWNED`", "$HOME", "%s", "a;b", "line\nnext"]
        secret = "secret $value %n ; literal"
        result = subprocess.run([*self.command, "--env", "TOKEN", "--", *argv],
                                env={**self.env, "TOKEN": secret}, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        request = self.request()
        self.assertEqual(request["payload"]["argv"], argv)
        self.assertEqual(request["payload"]["cwd"], str(self.root))
        self.assertEqual(request["payload"]["env"]["TOKEN"], secret)
        self.assertNotIn(secret, json.dumps(request["args"]) + result.stdout + result.stderr)
        self.assertFalse((self.root / "OWNED").exists())
        self.assertEqual(next((self.root / "logs").iterdir()).stat().st_mode & 0o777, 0o700)

    def test_unsupported_systemd_never_executes_command(self):
        sentinel = self.root / "executed"
        (self.root / "systemd-run").write_text("#!/bin/sh\nexit 1\n")
        result = subprocess.run([*self.command, "--", "touch", str(sentinel)],
                                env=self.env, capture_output=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(sentinel.exists())

    def test_missing_systemd_fails_closed(self):
        with patch.object(runner.shutil, "which", return_value=None):
            with self.assertRaises(ValueError):
                runner.run(runner.parse_args(["--", "touch", str(self.root / "executed")]))
        self.assertFalse((self.root / "executed").exists())

    def test_real_bootstrap_refuses_workload_in_inherited_scope(self):
        sentinel = self.root / "executed"
        payload = {"unit": "workspacer-release-check-unlaunched.service",
                   "argv": ["touch", str(sentinel)], "env": {}, "cwd": str(self.root)}
        result = subprocess.run([sys.executable, "-I", "-c", runner.CHILD],
                                input=json.dumps(payload), capture_output=True, text=True)
        self.assertEqual(result.returncode, 125)
        self.assertIn("isolation/exec failed", result.stderr)
        self.assertFalse(sentinel.exists())

    def test_all_operator_overrides_reach_properties_and_guard(self):
        result = self.launch(["--memory-high", "64M", "--memory-max", "128M",
                              "--memory-swap-max", "8M", "--cpu-quota", "25",
                              "--tasks-max", "16", "--timeout", "10",
                              "--env", "GOMEMLIMIT=32MiB"])
        self.assertEqual(result.returncode, 0)
        request = self.request()
        self.assertEqual(request["payload"]["limits"], {
            "memory.high": 67108864, "memory.max": 134217728, "memory.swap.max": 8388608,
            "memory.oom.group": 1, "pids.max": 16,
        })
        self.assertEqual(request["payload"]["cpu_quota"], 25)
        self.assertEqual(request["payload"]["env"]["GOMEMLIMIT"], "32MiB")
        for value in ("MemoryHigh=67108864", "MemoryMax=134217728", "MemorySwapMax=8388608",
                      "CPUQuota=25%", "TasksMax=16", "RuntimeMaxSec=10s"):
            self.assertIn("--property=" + value, request["args"])

    def test_launch_exception_preserves_failure_result(self):
        args = runner.parse_args(["--log-dir", str(self.root / "logs"), "--", "ignored"])
        with patch.object(runner.subprocess, "Popen", side_effect=OSError("failed")):
            with self.assertRaises(OSError):
                runner.run(args)
        self.assertEqual(self.result()["exitCode"], 125)

    def test_cancellation_only_stops_unique_unit_even_during_start(self):
        process = subprocess.Popen([*self.command, "--", "ignored"],
                                   env={**self.env, "FAKE_WAIT": "1"},
                                   stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        try:
            deadline = time.monotonic() + 5
            while not (self.root / "request.json").exists():
                self.assertLess(time.monotonic(), deadline)
                time.sleep(0.01)
            process.send_signal(signal.SIGINT)
            _, errors = process.communicate(timeout=10)
            self.assertEqual(process.returncode, 130, errors)
            calls = [json.loads(line) for line in (self.root / "control.jsonl").read_text().splitlines()]
            unit = self.request()["payload"]["unit"]
            self.assertTrue(unit.startswith("workspacer-release-check-"))
            self.assertTrue((self.root / "stopped").exists())
            for call in calls:
                self.assertIn(unit, call)
                self.assertNotIn("kill", call)
                self.assertFalse(any("app-workspacer" in arg for arg in call))
            self.assertEqual(self.result()["cancelledSignal"], signal.SIGINT)
        finally:
            if process.poll() is None:
                process.kill()  # only this disposable fake launcher
                process.wait()


if __name__ == "__main__":
    unittest.main()
