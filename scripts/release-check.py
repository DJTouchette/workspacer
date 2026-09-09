#!/usr/bin/env python3
"""Run one explicit Linux release command in a verified, capped user service."""

import argparse
import base64
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import uuid


DEFAULT_ENV = (
    "PATH", "HOME", "USER", "LOGNAME", "SHELL", "LANG", "LC_ALL", "LC_CTYPE",
    "TMPDIR", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_STATE_HOME",
    "XDG_CACHE_HOME", "XDG_RUNTIME_DIR", "APPDATA", "USERPROFILE",
)

# The only service command is a fixed bootstrap. User paths, argv and env travel
# on an anonymous input file, never through shell/systemd expansion or metadata.
# Keep this independent of the checkout path and of the manager's Python env.
CHILD = r'''
import json, os, pathlib, sys

def verify(payload, membership, root):
    lines = membership.strip().splitlines()
    if len(lines) != 1 or not lines[0].startswith("0::/"):
        raise ValueError("unified cgroup v2 is required")
    group = lines[0][3:]
    parts = pathlib.PurePosixPath(group).parts
    if parts[-1] != payload["unit"] or any(p.startswith("app-workspacer-") for p in parts):
        raise ValueError("command is not in its dedicated release service")
    if ".." in parts:
        raise ValueError("invalid cgroup path")
    directory = root / group.lstrip("/")
    actual = {name: (directory / name).read_text().strip() for name in (
        "memory.high", "memory.max", "memory.swap.max", "memory.oom.group",
        "cpu.max", "pids.max",
    )}
    for name, value in payload["limits"].items():
        if actual[name] != str(value):
            raise ValueError("cgroup limit not enforced: " + name)
    quota, period = actual["cpu.max"].split()
    if quota == "max" or int(quota) * 100 != int(period) * payload["cpu_quota"]:
        raise ValueError("CPU quota not enforced")
    return {"cgroup": group, "limits": actual}

def main():
    try:
        payload = json.load(sys.stdin)
        evidence = verify(payload, pathlib.Path("/proc/self/cgroup").read_text(),
                          pathlib.Path("/sys/fs/cgroup"))
        print("release-check verified: " + json.dumps(evidence, sort_keys=True), flush=True)
        os.chdir(payload["cwd"])
        # Check commands are noninteractive; stdin must not expose the payload.
        with open(os.devnull, "rb") as null:
            os.dup2(null.fileno(), 0)
        os.execvpe(payload["argv"][0], payload["argv"], payload["env"])
    except Exception as error:
        # Exception text could contain a credential-bearing argv/path. Do not log it.
        print("release-check: isolation/exec failed (" + type(error).__name__ + ")",
              file=sys.stderr, flush=True)
        return 125

if __name__ == "__main__":
    sys.exit(main())
'''


def memory(value):
    match = re.fullmatch(r"([0-9]+)([KMG]?)", value)
    if not match:
        raise argparse.ArgumentTypeError("use integer bytes or K/M/G, never infinity")
    return int(match[1]) * 1024 ** (" KMG".index(match[2]) if match[2] else 0)


def positive(value):
    if not value.isdigit() or int(value) < 1:
        raise argparse.ArgumentTypeError("must be a positive integer")
    return int(value)


def parse_args(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--memory-high", type=memory, default=memory("1536M"))
    parser.add_argument("--memory-max", type=memory, default=memory("2G"))
    parser.add_argument("--memory-swap-max", type=memory, default=0)
    parser.add_argument("--cpu-quota", type=positive, default=100, help="integer percent")
    parser.add_argument("--tasks-max", type=positive, default=128)
    parser.add_argument("--timeout", type=positive, default=3600, help="seconds")
    parser.add_argument("--cwd", type=Path, default=Path.cwd())
    parser.add_argument("--log-dir", type=Path, default=Path(".workspacer/release-checks"))
    parser.add_argument("--env", action="append", default=[], metavar="NAME[=VALUE]",
                        help="explicitly copy a caller variable or set a value")
    parser.add_argument("command", nargs=argparse.REMAINDER, help="-- command arg ...")
    args = parser.parse_args(argv)
    if args.command[:1] == ["--"]:
        args.command.pop(0)
    if not args.command:
        parser.error("an explicit command is required after --")
    if not 0 < args.memory_high <= args.memory_max:
        parser.error("require 0 < memory-high <= memory-max")
    args.cwd = args.cwd.resolve(strict=True)
    if not args.cwd.is_dir():
        parser.error("cwd must be a directory")
    return args


def environment(overrides):
    env = {key: os.environ[key] for key in DEFAULT_ENV if key in os.environ}
    env.update(GOMEMLIMIT="1GiB", GOMAXPROCS="2", CARGO_BUILD_JOBS="1")
    for item in overrides:
        name, separator, value = item.partition("=")
        if not re.fullmatch(r"[A-Za-z_][A-Za-z_0-9]*", name):
            raise ValueError("invalid environment variable name")
        if not separator:
            if name not in os.environ:
                raise ValueError("requested environment variable is unset")
            value = os.environ[name]
        if "\0" in value:
            raise ValueError("NUL in environment value")
        env[name] = value
    return env


def service_command(args, unit, systemd_run):
    properties = {
        "MemoryAccounting": "yes", "CPUAccounting": "yes", "TasksAccounting": "yes",
        "MemoryHigh": args.memory_high, "MemoryMax": args.memory_max,
        "MemorySwapMax": args.memory_swap_max, "CPUQuota": str(args.cpu_quota) + "%",
        "TasksMax": args.tasks_max, "OOMPolicy": "kill",
        "KillMode": "control-group", "TimeoutStopSec": "5s",
        "TimeoutStartSec": "30s", "RuntimeMaxSec": str(args.timeout) + "s",
    }
    bootstrap = "import base64;exec(base64.b64decode(" + repr(
        base64.b64encode(CHILD.encode()).decode()) + "))"
    return [systemd_run, "--user", "--no-ask-password", "--wait", "--pipe",
            "--collect", "--service-type=exec", "--expand-environment=no",
            "--slice=app.slice", "--description=Workspacer release check", "--unit=" + unit,
            *["--property=" + key + "=" + str(value) for key, value in properties.items()],
            "--", "/usr/bin/python3", "-I", "-c", bootstrap]


def stop_unit(systemctl, unit, log):
    # Never signal a PID, process group, wildcard, slice, or caller unit.
    try:
        return subprocess.run(
            [systemctl, "--user", "--no-ask-password", "stop", unit],
            stdin=subprocess.DEVNULL, stdout=log, stderr=log, timeout=10,
            start_new_session=True, check=False,
        ).returncode == 0
    except (OSError, subprocess.TimeoutExpired):
        return False


def unit_gone(systemctl, unit):
    try:
        state = subprocess.run(
            [systemctl, "--user", "show", unit, "--property=LoadState", "--value"],
            capture_output=True, timeout=10, check=False, start_new_session=True,
        )
        return state.stdout.strip() == b"not-found"
    except (OSError, subprocess.TimeoutExpired):
        return False


def run(args):
    env = environment(args.env)
    systemd_run, systemctl = shutil.which("systemd-run"), shutil.which("systemctl")
    if (sys.platform != "linux" or not systemd_run or not systemctl
            or not Path("/usr/bin/python3").is_file()
            or not Path("/sys/fs/cgroup/cgroup.controllers").is_file()):
        raise ValueError("Linux, Python 3, user systemd and cgroup v2 are required")
    unit = "workspacer-release-check-" + uuid.uuid4().hex + ".service"
    args.log_dir.mkdir(parents=True, exist_ok=True)
    directory = Path(tempfile.mkdtemp(prefix=unit[:-8] + "-", dir=args.log_dir)).resolve()
    print("release-check unit: " + unit, flush=True)
    print("release-check logs: " + str(directory), flush=True)
    payload = {
        "unit": unit, "cwd": str(args.cwd), "argv": args.command, "env": env,
        "cpu_quota": args.cpu_quota,
        "limits": {"memory.high": args.memory_high, "memory.max": args.memory_max,
                   "memory.swap.max": args.memory_swap_max, "memory.oom.group": 1,
                   "pids.max": args.tasks_max},
    }
    cancelled = 0

    def cancel(signum, _frame):
        nonlocal cancelled
        cancelled = cancelled or signum

    previous = {sig: signal.signal(sig, cancel) for sig in (signal.SIGINT, signal.SIGTERM)}
    process = None
    code = 125
    cleanup_ok = True
    try:
        with (directory / "output.log").open("xb") as log, tempfile.TemporaryFile(dir=directory) as data:
            data.write(json.dumps(payload).encode())
            data.seek(0)
            try:
                process = subprocess.Popen(service_command(args, unit, systemd_run),
                                           stdin=data, stdout=log, stderr=log,
                                           start_new_session=True)
                deadline = time.monotonic() + args.timeout + 45
                cancel_deadline = None
                while True:
                    try:
                        code = process.wait(timeout=0.2)
                        break
                    except subprocess.TimeoutExpired:
                        if time.monotonic() > deadline:
                            code = 125
                            break
                        if cancelled:
                            if cancel_deadline is None:
                                cancel_deadline = time.monotonic() + 45
                            # Retry until the launcher completes: an early stop can
                            # race creation of the transient service's start job.
                            cleanup_ok = stop_unit(systemctl, unit, log)
                            if time.monotonic() > cancel_deadline:
                                code = 125
                                break
                if cancelled:
                    code = 128 + cancelled
                elif code < 0:
                    code = 128 - code
            finally:
                # On normal completion systemd already reaps the service's children.
                # An error/disconnected launcher can leave it running; stop only ours.
                if process is not None and (cancelled or code != 0):
                    cleanup_ok = unit_gone(systemctl, unit) or stop_unit(systemctl, unit, log)
                    try:
                        process.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        cleanup_ok = False
                if not cleanup_ok:
                    code = 125
    finally:
        for sig, handler in previous.items():
            signal.signal(sig, handler)
        result = {"unit": unit, "exitCode": code, "cancelledSignal": cancelled,
                  "cleanupConfirmed": cleanup_ok}
        (directory / "result.json").write_text(json.dumps(result, indent=2) + "\n")
        print("release-check exit: " + str(code), flush=True)
        if not cleanup_ok:
            print("release-check: cleanup unconfirmed; inspect only " + unit, file=sys.stderr)
    return code


def main():
    try:
        return run(parse_args())
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print("release-check: refused/failed (" + type(error).__name__
              + "); no inherited-scope fallback. See --help and docs/release-checks.md.",
              file=sys.stderr)
        return 125


if __name__ == "__main__":
    sys.exit(main())
