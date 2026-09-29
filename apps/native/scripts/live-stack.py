#!/usr/bin/env python3
"""Opt-in real-provider test on an isolated Workspacer backend.

Uses fresh config, SQLite state, plugin directory and loopback ports. Skips
global hook installation. Provider account credentials remain the caller's.
Never connects to the caller's existing hub or rotates its credentials.
"""
import argparse
import json
import os
from pathlib import Path
import socket
import signal
import subprocess
import tempfile
import time

from smoke import screenshot


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def stop(process):
    if process is None or process.poll() is not None:
        return
    process.terminate()
    try:
        process.wait(timeout=15)
    except subprocess.TimeoutExpired:
        # Each process below owns its own process group. Never signal the
        # existing backend or the invoking shell's group during cleanup.
        os.killpg(process.pid, signal.SIGKILL)
        process.wait()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--backend", default="workspacer-rust", help="Rust standalone backend executable")
    parser.add_argument("--harness", type=Path, required=True)
    parser.add_argument("--native", type=Path, help="Also render the live session in an X11 window")
    parser.add_argument("--provider", default="claude")
    parser.add_argument("--output", type=Path, default=Path("native-live.png"))
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="wks-native-live-") as scratch:
        root = Path(scratch)
        for name in ("config", "data", "plugins", "workspace"):
            (root / name).mkdir()
        # Keep ports distinct. The launcher detects races/collisions and refuses
        # them rather than stopping an incumbent service.
        ports = set()
        while len(ports) < 4:
            ports.add(free_port())
        api, hooks, hub, facade = sorted(ports)
        env = os.environ.copy()
        env["XDG_CONFIG_HOME"] = str(root / "config")
        env["XDG_DATA_HOME"] = str(root / "data")
        env.pop("HUB_TOKEN", None)
        env.pop("WORKSPACER_ALLOW_NEW_TOKEN", None)
        url = f"ws://127.0.0.1:{hub}/bus"
        token_file = root / "config/workspacer/remote-token"
        stack = native = None
        # The ready banner contains the test stack's token. Keep it private,
        # never relay it to terminal logs or report artifacts.
        with (root / "ready.log").open("w+") as ready, (root / "backend.log").open("w+") as logs:
            try:
                stack = subprocess.Popen([
                    args.backend, "--config-dir", str(root / "config/workspacer"),
                    "serve", "--json", "--no-claudemon-init",
                    "--mcp-port", str(facade), "--data-dir", str(root / "data"),
                    "--claudemon-api-port", str(api), "--claudemon-hook-port", str(hooks),
                    "--hub-port", str(hub), "--claudemon-db-path", str(root / "sessions.db"),
                    "--plugins-dir", str(root / "plugins")
                ], env=env, stdout=ready, stderr=logs, start_new_session=True)
                deadline = time.monotonic() + 45
                while time.monotonic() < deadline:
                    if stack.poll() is not None:
                        raise RuntimeError(f"Isolated backend exited ({stack.returncode}); no existing backend was changed")
                    ready.seek(0)
                    if token_file.exists() and ready.read().strip():
                        break
                    time.sleep(0.2)
                else:
                    raise RuntimeError("Isolated backend did not become ready")
                # Verify the complete backend capability graph before spawning.
                # Retry only a read here; never retry an uncertain spawn.
                deadline = time.monotonic() + 60
                last_probe = "No provider response"
                while time.monotonic() < deadline:
                    probe = subprocess.run([
                        str(args.harness.resolve()), "probe", "--bus", url,
                        "--token-file", str(token_file)
                    ], env=env, capture_output=True, text=True, timeout=25)
                    if probe.returncode == 0:
                        break
                    last_probe = probe.stderr.strip()
                    time.sleep(1)
                else:
                    raise RuntimeError(f"Isolated provider did not become ready: {last_probe}")
                command = [str(args.harness.resolve()), "live", "--bus", url,
                           "--token-file", str(token_file), "--provider", args.provider,
                           "--cwd", str(root / "workspace")]
                if args.native:
                    command.append("--keep-open")
                result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=300)
                if result.returncode:
                    # Harness errors never include credential material.
                    raise RuntimeError(result.stderr.strip())
                report = json.loads(result.stdout)
                if args.native:
                    native = subprocess.Popen([
                        str(args.native.resolve()), "--bus", url, "--token-file", str(token_file),
                        "--session", report["session_id"]
                    ], env=env, stdout=subprocess.DEVNULL, start_new_session=True)
                    window = subprocess.check_output([
                        "xdotool", "search", "--sync", "--onlyvisible", "--pid", str(native.pid)
                    ], timeout=20, text=True).splitlines()[0]
                    subprocess.run(["xdotool", "windowfocus", window, "windowsize", window, "1000", "700"], check=True)
                    time.sleep(5)
                    report["native_window_colors"] = screenshot(int(window), 1000, 700, args.output)
                    report["native_screenshot"] = str(args.output)
                report["isolated_backend"] = True
                report["backend_shutdown_on_exit"] = True
                print(json.dumps(report, indent=2))
            finally:
                stop(native)
                stop(stack)


if __name__ == "__main__":
    main()
