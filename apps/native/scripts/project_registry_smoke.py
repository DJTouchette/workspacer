#!/usr/bin/env python3
"""Private Linux/X11 project-registry ordering fixture (no providers).

Starts its own Xvfb, Openbox, fake loopback hub and native binary with a fresh
HOME and an explicit environment. Requires paths to test display tools and a
lavapipe ICD. Saves request/response evidence, binary identity and screenshots;
never attaches to an existing display or touches the user's desktop/config.
"""
import argparse
import asyncio
import base64
import hashlib
import json
import os
from pathlib import Path
import signal
import struct
import subprocess
import sys
import time


def fake_hub(root):
    state = {"projects": {"/work/api": {"label": "API Server", "favourite": True},
                          "/work/web/": {"lastOpened": 1790000000000}},
             "directories": {"favourites": [], "recent": []}}
    saves = 0

    def record(kind, value):
        with (root / "requests.jsonl").open("a") as log:
            log.write(json.dumps({"kind": kind, **value}) + "\n")

    async def client(reader, writer):
        nonlocal saves
        pending = set()
        try:
            headers = (await reader.readuntil(b"\r\n\r\n")).decode()
            key = next(line.split(":", 1)[1].strip() for line in headers.split("\r\n")
                       if line.lower().startswith("sec-websocket-key:"))
            accept = base64.b64encode(hashlib.sha1(
                (key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode()).digest()).decode()
            writer.write(("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n"
                          "Connection: Upgrade\r\nSec-WebSocket-Accept: " + accept + "\r\n\r\n").encode())

            async def send(obj, opcode=1):
                data = json.dumps(obj).encode() if opcode == 1 else obj
                head = bytes([128 | opcode, len(data)]) if len(data) < 126 else bytes([128 | opcode, 126]) + struct.pack("!H", len(data))
                writer.write(head + data)
                await writer.drain()
                if opcode == 1:
                    record("response", obj)

            async def save(req, number):
                while not (root / f"release-{number}").exists():
                    await asyncio.sleep(.02)
                state.update(req["params"])
                (root / "saved.json").write_text(json.dumps(state, indent=2))
                await send({"op": "result", "id": req["id"], "result": state})

            await send({"op": "hello", "scope": "operator"})
            while True:
                a, b = await reader.readexactly(2)
                length = b & 127
                if length == 126:
                    length = struct.unpack("!H", await reader.readexactly(2))[0]
                elif length == 127:
                    length = struct.unpack("!Q", await reader.readexactly(8))[0]
                mask = await reader.readexactly(4) if b & 128 else b""
                data = await reader.readexactly(length)
                if mask:
                    data = bytes(v ^ mask[i % 4] for i, v in enumerate(data))
                if a & 15 == 8:
                    break
                if a & 15 == 9:
                    await send(data, 10)
                    continue
                if a & 15 != 1:
                    continue
                req = json.loads(data)
                record("request", req)
                if req.get("op") != "call":
                    continue
                method = req.get("method")
                out = {}
                if method == "sessions.snapshots":
                    out = []
                elif method == "usage.report":
                    out = {"providers": []}
                elif method == "config.get":
                    out = state
                elif method == "config.save":
                    saves += 1
                    task = asyncio.create_task(save(req, saves))
                    pending.add(task)
                    task.add_done_callback(pending.discard)
                    continue
                elif method == "fs.listDir":
                    out = {"path": req.get("params", {}).get("path") or "/work", "parent": "/", "home": "/work", "dirs": []}
                elif method == "git.status":
                    out = {"branch": "main", "files": []}
                elif method == "claude.listModels":
                    out = {"aliases": [{"model": "opus", "label": "Opus"}]}
                elif method == "agents.spawn":
                    await send({"op": "error", "id": req["id"], "error": "fixture refuses all launches"})
                    continue
                await send({"op": "result", "id": req["id"], "result": out})
        except (asyncio.IncompleteReadError, ConnectionResetError):
            pass
        finally:
            for task in pending:
                task.cancel()
            writer.close()

    async def serve():
        server = await asyncio.start_server(client, "127.0.0.1", 0)
        (root / "bus-port").write_text(str(server.sockets[0].getsockname()[1]))
        async with server:
            await server.serve_forever()
    asyncio.run(serve())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ["binary", "output", "xvfb", "openbox", "openbox-config", "xdotool", "vulkan-icd"]:
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--library-path", default="")
    parser.add_argument("--data-dirs", default="/usr/share")
    parser.add_argument("--display", type=int, default=254)
    args = parser.parse_args()
    root = args.output.resolve()
    root.mkdir(parents=True, exist_ok=False)
    for name in ["home", "config", "runtime", "bin", "shots"]:
        (root / name).mkdir(mode=0o700)
    display = f":{args.display}"
    assert not Path(f"/tmp/.X11-unix/X{args.display}").exists()
    assert not Path(f"/tmp/.X{args.display}-lock").exists()
    env = {"LANG": "C.UTF-8", "DISPLAY": display, "HOME": str(root / "home"),
           "XDG_CONFIG_HOME": str(root / "config"), "XDG_RUNTIME_DIR": str(root / "runtime"),
           "XDG_DATA_DIRS": args.data_dirs, "DBUS_SESSION_BUS_ADDRESS": "unix:path=/nonexistent",
           "PATH": str(root / "bin"), "LD_LIBRARY_PATH": args.library_path,
           "VK_DRIVER_FILES": str(args.vulkan_icd.resolve()), "RUST_LOG": "warn"}
    processes = []

    def start(name, command):
        process = subprocess.Popen(list(map(str, command)), env=env,
                                   stdout=(root / f"{name}.log").open("w"),
                                   stderr=subprocess.STDOUT, start_new_session=True)
        processes.append((name, process))
        (root / "pids.json").write_text(json.dumps([{ "name": n, "pid": p.pid} for n, p in processes]))
        return process

    def wait_for(predicate):
        end = time.monotonic() + 20
        while time.monotonic() < end:
            if predicate():
                return
            time.sleep(.1)
        raise RuntimeError("fixture condition timed out")

    def x(*command):
        return subprocess.run([str(args.xdotool), *map(str, command)], env=env,
                              text=True, capture_output=True, check=True).stdout.strip()

    def calls():
        return [r for line in (root / "requests.jsonl").read_text().splitlines()
                if (r := json.loads(line)).get("kind") == "request" and r.get("op") == "call"]

    try:
        identity = {"commit": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
                    "binary": str(args.binary.resolve()),
                    "sha256": hashlib.sha256(args.binary.read_bytes()).hexdigest()}
        (root / "identity.json").write_text(json.dumps(identity, indent=2))
        start("hub", [sys.executable, Path(__file__).resolve(), "--fake-hub", root])
        wait_for(lambda: (root / "bus-port").exists())
        start("xvfb", [args.xvfb, display, "-screen", "0", "1800x1100x24", "-nolisten", "tcp"])
        wait_for(lambda: Path(f"/tmp/.X11-unix/X{args.display}").exists())
        start("wm", [args.openbox, "--config-file", args.openbox_config])
        time.sleep(1)
        start("app", [args.binary.resolve(), "--bus", "ws://127.0.0.1:" + (root / "bus-port").read_text() + "/bus"])
        time.sleep(5)
        window = x("search", "--onlyvisible", "--name", "^Workspacer Native$").splitlines()[0]
        x("windowsize", window, 1000, 800)
        x("windowfocus", window)
        time.sleep(1)

        def click(px, py):
            x("mousemove", "--window", window, px, py)
            time.sleep(.15)
            x("mousedown", 1)
            time.sleep(.12)
            x("mouseup", 1)
            time.sleep(.3)

        def shot(name):
            # Capture in a fresh interpreter so ctypes opens only our display.
            subprocess.run([sys.executable, Path(__file__).resolve(), "--shot", window,
                            str(root / "shots" / name)], env=env, check=True)

        x("key", "--delay", 100, "ctrl+n")
        time.sleep(2)
        shot("picker.png")
        # Coordinates are pinned to this fixture's 1000x800 window.
        # Select web (the second row), then pin from its summary.
        click(500, 262)
        shot("selected.png")
        click(868, 176)
        wait_for(lambda: len([r for r in calls() if r["method"] == "config.save"]) == 1)
        click(927, 176)  # Change triggers the overlapping Projects refresh.
        time.sleep(.5)
        pending = calls()
        save_index = next(i for i, r in enumerate(pending) if r["method"] == "config.save")
        assert not any(r["method"] == "config.get" for r in pending[save_index + 1:]), "refresh escaped transaction barrier"
        shot("pending-pin-refresh.png")
        (root / "release-1").touch()
        wait_for(lambda: any(r["method"] == "config.get" for r in calls()[save_index + 1:]))
        time.sleep(1)
        shot("pinned-picker.png")
        assert json.loads((root / "saved.json").read_text())["projects"]["/work/web/"]["favourite"] is True
        click(927, 163)  # Cancel returns to the selected summary.
        shot("pinned-summary.png")
        click(868, 176)  # The same star now requests Unpin.
        wait_for(lambda: len([r for r in calls() if r["method"] == "config.save"]) == 2)
        click(927, 176)
        time.sleep(.5)
        pending = calls()
        save_index = max(i for i, r in enumerate(pending) if r["method"] == "config.save")
        assert pending[save_index]["params"]["projects"]["/work/web/"]["favourite"] is False
        assert not any(r["method"] == "config.get" for r in pending[save_index + 1:])
        (root / "release-2").touch()
        wait_for(lambda: any(r["method"] == "config.get" for r in calls()[save_index + 1:]))
        time.sleep(1)
        shot("unpinned-picker.png")
        final = json.loads((root / "saved.json").read_text())
        assert final["projects"]["/work/web/"]["favourite"] is False
        assert final["projects"]["/work/api"]["label"] == "API Server"
        assert not any(r["method"] == "agents.spawn" for r in calls())
        (root / "result.json").write_text(json.dumps({"passed": True, **identity}, indent=2))
    finally:
        for _, process in reversed(processes):
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGTERM)
        for _, process in reversed(processes):
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=5)
        (root / "cleanup.json").write_text(json.dumps({"exitCodes": {n: p.returncode for n, p in processes},
            "displaySocketRemoved": not Path(f"/tmp/.X11-unix/X{args.display}").exists()}, indent=2))


if __name__ == "__main__":
    if sys.argv[1:2] == ["--fake-hub"]:
        fake_hub(Path(sys.argv[2]))
    elif sys.argv[1:2] == ["--shot"]:
        from smoke import screenshot
        screenshot(int(sys.argv[2]), 1000, 800, Path(sys.argv[3]))
    else:
        main()
