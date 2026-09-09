#!/usr/bin/env python3
"""Opt-in, bounded real-systemd validation; never starts Workspacer or a suite."""

import argparse
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time


RUNNER = Path(__file__).with_name("release-check.py").resolve()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--memory-ceiling", action="store_true",
                        help="also attempt a finite 96 MiB allocation inside a verified 64 MiB service")
    args = parser.parse_args()
    root = Path(".workspacer/release-check-smoke").resolve()
    root.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="cwd $literal %s ", dir=root) as scratch:
        common = [sys.executable, str(RUNNER), "--log-dir", str(root), "--cwd", scratch,
                  "--timeout", "15"]
        literal = ["", "$HOME", "$(touch OWNED)", "`touch OWNED`", "%s", "a;b", "line\nnext"]
        code = (
            "import json,os,sys; "
            "assert sys.argv[1:] == json.loads(os.environ['EXPECTED_ARGS']); "
            "assert os.getcwd() == os.environ['EXPECTED_CWD']; "
            "assert os.environ['GOMEMLIMIT'] == '1GiB'; "
            "print('literal argv, cwd and environment verified'); sys.exit(37)"
        )
        program = Path(scratch) / "program with $spaces %s"
        program.write_text("#!/usr/bin/python3\n" + code + "\n")
        program.chmod(0o700)
        result = subprocess.run([*common, "--env", "EXPECTED_ARGS", "--env", "EXPECTED_CWD",
                                 "--", str(program), *literal],
                                env={**os.environ, "EXPECTED_ARGS": json.dumps(literal),
                                     "EXPECTED_CWD": scratch}, check=False, timeout=30)
        assert result.returncode == 37, "isolated smoke did not propagate exit 37"
        assert not (Path(scratch) / "OWNED").exists()
        result = subprocess.run([*common, "--", "/usr/bin/true"], check=False, timeout=30)
        assert result.returncode == 0, "isolated success failed"

        marker = Path(scratch) / "ready"
        before = set(root.glob("*/result.json"))
        process = subprocess.Popen([*common, "--", "/usr/bin/python3", "-c",
                                    "import pathlib,time; pathlib.Path('ready').touch(); time.sleep(10)"],
                                   start_new_session=True)
        try:
            deadline = time.monotonic() + 10
            while not marker.exists():
                assert process.poll() is None and time.monotonic() < deadline, "service did not start"
                time.sleep(0.05)
            process.send_signal(signal.SIGTERM)  # only our disposable runner
            assert process.wait(timeout=25) == 143, "cancellation status lost"
            results = set(root.glob("*/result.json")) - before
            assert len(results) == 1
            record = json.loads(results.pop().read_text())
            assert record["cleanupConfirmed"], "service cleanup unconfirmed"
            state = subprocess.run(["systemctl", "--user", "show", record["unit"],
                                    "--property=LoadState", "--value"],
                                   capture_output=True, text=True, check=False, timeout=10)
            assert state.stdout.strip() == "not-found", "cancelled service still loaded"
        finally:
            if process.poll() is None:
                process.send_signal(signal.SIGTERM)
                process.wait(timeout=30)

        if args.memory_ceiling:
            # A bounded allocation, never an unbounded stress loop. The runner's
            # guard verifies ALL controllers before this command can start. Check
            # the tiny ceiling again and available RAM immediately before allocating.
            code = '''
import pathlib
group = pathlib.Path('/proc/self/cgroup').read_text().strip().split('::')[1]
root = pathlib.Path('/sys/fs/cgroup') / group.lstrip('/')
assert (root / 'memory.max').read_text().strip() == '67108864'
assert (root / 'memory.swap.max').read_text().strip() == '0'
assert (root / 'memory.oom.group').read_text().strip() == '1'
info = dict(line.split(':', 1) for line in pathlib.Path('/proc/meminfo').read_text().splitlines())
assert int(info['MemAvailable'].split()[0]) >= 512 * 1024, 'insufficient spare RAM for smoke'
print('tiny ceiling verified; attempting only 96 MiB', flush=True)
data = bytearray(96 * 1024 * 1024)
raise RuntimeError('64 MiB ceiling did not kill the 96 MiB allocation')
'''
            before = set(root.glob("*/output.log"))
            result = subprocess.run([*common, "--memory-high", "64M", "--memory-max", "64M",
                                     "--cpu-quota", "25", "--tasks-max", "16",
                                     "--", "/usr/bin/python3", "-c", code],
                                    check=False, timeout=30)
            logs = set(root.glob("*/output.log")) - before
            assert result.returncode != 0 and len(logs) == 1, "memory ceiling did not fail"
            assert "oom-kill" in logs.pop().read_text(), "OOM kill unavailable or not verified"
    print("Real isolation smoke passed; evidence retained in " + str(root))


if __name__ == "__main__":
    main()
