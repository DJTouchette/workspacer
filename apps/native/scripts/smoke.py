#!/usr/bin/env python3
"""Linux/X11 native-window smoke and measurement; no Python dependencies.

Run under an existing display or xvfb-run. Uses xdotool for real input and
XGetImage for pixels, so a window that merely exists but paints black fails.
The default --demo workload includes the in-process fixture in RSS/CPU.
"""
import argparse
import ctypes as C
import ctypes.util
import json
import os
from pathlib import Path
import struct
import subprocess
import time
import tempfile
import zlib


class XImage(C.Structure):
    _fields_ = [(name, C.c_int) for name in ("width", "height", "xoffset", "format")] + [
        ("data", C.c_void_p)
    ] + [(name, C.c_int) for name in (
        "byte_order", "bitmap_unit", "bitmap_bit_order", "bitmap_pad", "depth",
        "bytes_per_line", "bits_per_pixel"
    )]


def screenshot(window, width, height, output, left=0, top=0):
    x = C.CDLL(ctypes.util.find_library("X11"))
    x.XOpenDisplay.argtypes = [C.c_char_p]
    x.XOpenDisplay.restype = C.c_void_p
    x.XGetImage.argtypes = [C.c_void_p, C.c_ulong, C.c_int, C.c_int, C.c_uint,
                           C.c_uint, C.c_ulong, C.c_int]
    x.XGetImage.restype = C.POINTER(XImage)
    x.XDestroyImage.argtypes = [C.POINTER(XImage)]
    x.XCloseDisplay.argtypes = [C.c_void_p]
    display = x.XOpenDisplay(None)
    if not display:
        raise RuntimeError("Cannot open DISPLAY")
    image = x.XGetImage(display, window, left, top, width, height, 0xFFFFFFFF, 2)
    if not image:
        x.XCloseDisplay(display)
        raise RuntimeError("Cannot capture native window")
    try:
        info = image.contents
        if info.bits_per_pixel != 32 or info.byte_order != 0:
            raise RuntimeError("Smoke capture expects a little-endian 32-bit X11 visual")
        raw = C.string_at(info.data, info.bytes_per_line * height)
        rows, colors = [], set()
        for y in range(height):
            row = bytearray([0])
            for col in range(width):
                index = y * info.bytes_per_line + col * 4
                rgb = bytes((raw[index + 2], raw[index + 1], raw[index]))
                row.extend(rgb)
                colors.add(rgb)
            rows.append(row)
        def chunk(tag, data):
            return (struct.pack(">I", len(data)) + tag + data
                    + struct.pack(">I", zlib.crc32(tag + data)))
        output.write_bytes(b"\x89PNG\r\n\x1a\n"
                           + chunk(b"IHDR", struct.pack(">2I5B", width, height, 8, 2, 0, 0, 0))
                           + chunk(b"IDAT", zlib.compress(b"".join(rows))) + chunk(b"IEND", b""))
        if len(colors) < 16:
            raise RuntimeError(f"Window painted only {len(colors)} colors; inspect {output}")
        return len(colors)
    finally:
        x.XDestroyImage(image)
        x.XCloseDisplay(display)


def process_sample(pid):
    fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
    cpu_seconds = (int(fields[11]) + int(fields[12])) / os.sysconf("SC_CLK_TCK")
    rss = int(fields[21]) * os.sysconf("SC_PAGE_SIZE")
    return cpu_seconds, rss


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/debug/wks-native"))
    parser.add_argument("--bus", help="Use a separately running fixture instead of --demo")
    parser.add_argument("--output", type=Path, default=Path("native-smoke.png"))
    parser.add_argument("--settle-seconds", type=float, default=3)
    parser.add_argument("--sample-seconds", type=float, default=3)
    parser.add_argument("--screen", choices=("conversation", "projects", "settings", "history", "changes", "setup", "session", "model"), default="conversation")
    parser.add_argument("--theme", choices=("dark", "light", "nord", "tokyo-night", "catppuccin", "gruvbox",
                                            "everforest", "catppuccin-latte"), default="dark")
    parser.add_argument("--width", type=int, default=1000)
    parser.add_argument("--height", type=int, default=700)
    parser.add_argument("--scroll-pages", type=int, default=0, help="Scroll chat upward by this many half-pages before capture")
    parser.add_argument("--no-input", action="store_true", help="Capture without sending a fixture message")
    parser.add_argument("--paint-region", action="append", default=[], metavar="X,Y,W,H",
                        help="Require a region to paint content, not a blank background; repeatable")
    parser.add_argument("--animation-region", help="Verify pixels change across four frames in x,y,width,height (for a visible spinner)")
    parser.add_argument("--click", action="append", help="Click x,y before capture; repeat for a sequence of preview and toggle checks")
    parser.add_argument("--drag", action="append",
                        help="Drag-select from x1,y1 to x2,y2 before capture (text selection checks); repeatable")
    parser.add_argument("--hover", help="Hover a window point x,y before capture (for control styling checks)")
    parser.add_argument("--keys", nargs="+", help="Additional X11 keys before capture, for example Escape Tab")
    parser.add_argument("--new-session", action="store_true", help="Capture the creation form; requires a fixture --bus")
    parser.add_argument("--session", help="Open this session ID at startup; requires a fixture --bus")
    parser.add_argument("--setting", action="append", default=[], metavar="KEY=JSON",
                        help="Seed native-settings.json, for example merge_turn_tools=true; repeatable")
    args = parser.parse_args()
    if args.width < 720 or args.height < 480:
        parser.error("The native minimum window size is 720 × 480")
    if args.new_session and not args.bus:
        parser.error("--new-session requires a fixture --bus; creation is disabled in demo mode")
    if args.session and not args.bus:
        parser.error("--session requires a fixture --bus")
    seeded = {}
    for setting in args.setting:
        key, _, value = setting.partition("=")
        try:
            seeded[key] = json.loads(value)
        except ValueError:
            parser.error("--setting requires KEY=JSON, for example merge_turn_tools=true")
    animation_region = None
    clicks = []
    for click in args.click or []:
        try:
            x, y = map(int, click.split(","))
            if not (0 <= x < args.width and 0 <= y < args.height):
                raise ValueError()
            clicks.append((x, y))
        except ValueError:
            parser.error("--click requires x,y inside the window")
    drags = []
    for drag in args.drag or []:
        try:
            x1, y1, x2, y2 = map(int, drag.split(","))
            if not all(0 <= x < args.width for x in (x1, x2)) or not all(0 <= y < args.height for y in (y1, y2)):
                raise ValueError()
            drags.append((x1, y1, x2, y2))
        except ValueError:
            parser.error("--drag requires x1,y1,x2,y2 inside the window")
    hover = None
    if args.hover:
        try:
            x, y = map(int, args.hover.split(","))
            if not (0 <= x < args.width and 0 <= y < args.height):
                raise ValueError()
            hover = (x, y)
        except ValueError:
            parser.error("--hover requires x,y inside the window")
    if args.animation_region:
        try:
            left, top, width, height = map(int, args.animation_region.split(","))
            if min(left, top) < 0 or min(width, height) <= 0 or left + width > args.width or top + height > args.height:
                raise ValueError()
            animation_region = (left, top, width, height)
        except ValueError:
            parser.error("--animation-region requires x,y,width,height inside the window")
    paint_regions = []
    for region in args.paint_region:
        try:
            left, top, width, height = map(int, region.split(","))
            if min(left, top) < 0 or min(width, height) <= 0 or left + width > args.width or top + height > args.height:
                raise ValueError()
            paint_regions.append((left, top, width, height))
        except ValueError:
            parser.error("--paint-region requires x,y,width,height inside the window")
    command = [str(args.binary.resolve())]
    command += ["--bus", args.bus] if args.bus else ["--demo"]
    command += ["--session", args.session] if args.session else []
    started = time.monotonic()
    # Isolate appearance from the user's real preference and credentials.
    settings = tempfile.TemporaryDirectory(prefix="wks-native-smoke-")
    config = Path(settings.name) / "workspacer"
    config.mkdir()
    (config / "native-theme.json").write_text(json.dumps(args.theme))
    if seeded:
        (config / "native-settings.json").write_text(json.dumps(seeded))
    environment = dict(os.environ, XDG_CONFIG_HOME=settings.name)
    process = subprocess.Popen(command, stdout=subprocess.DEVNULL, env=environment)
    try:
        window = subprocess.check_output(
            ["xdotool", "search", "--sync", "--onlyvisible", "--pid", str(process.pid)],
            timeout=20, text=True).splitlines()[0]
        appeared_ms = (time.monotonic() - started) * 1000
        def drive(*commands):
            subprocess.run(["xdotool", *commands], check=True, timeout=5)
        drive("windowfocus", window, "windowsize", window, str(args.width), str(args.height))
        time.sleep(args.settle_seconds)
        before, _ = process_sample(process.pid)
        idle_start = time.monotonic()
        time.sleep(args.sample_seconds)
        after, rss = process_sample(process.pid)
        idle_cpu = 100 * (after - before) / (time.monotonic() - idle_start)
        if args.new_session:
            drive("key", "ctrl+n")
        elif not args.no_input:
            drive("key", "ctrl+l")
            drive("type", "Native window smoke test")
            drive("key", "ctrl+Return")
        if args.screen != "conversation":
            drive("key", "Escape")
            drive("key", "--delay", "80", "g", {"projects": "p", "settings": "s", "history": "h", "changes": "d", "setup": "a", "session": "e", "model": "m"}[args.screen])
        if args.scroll_pages:
            drive("key", "Escape")
            drive("key", "--delay", "80", *(["ctrl+u"] * min(50, max(0, args.scroll_pages))))
        time.sleep(1)
        for click in clicks:
            drive("mousemove", "--window", window, str(click[0]), str(click[1]), "click", "1")
            # Capture the resulting layout, without a tooltip left behind by
            # the button that moved or disappeared after the click.
            drive("mousemove", "--window", window, str(args.width - 1), str(args.height - 1))
            time.sleep(1)
        if args.keys:
            drive("key", "--delay", "80", *args.keys)
            time.sleep(1)
        for x1, y1, x2, y2 in drags:
            # Stepped moves: GPUI extends a selection from mouse-move events.
            drive("mousemove", "--window", window, str(x1), str(y1), "mousedown", "1")
            for step in range(1, 9):
                drive("mousemove", "--window", window,
                      str(x1 + (x2 - x1) * step // 8), str(y1 + (y2 - y1) * step // 8))
                time.sleep(0.05)
            drive("mouseup", "1")
            time.sleep(1)
        if hover:
            drive("mousemove", "--window", window, str(hover[0]), str(hover[1]))
            time.sleep(1)
        colors = screenshot(int(window), args.width, args.height, args.output)
        region_colors = []
        for index, (left, top, width, height) in enumerate(paint_regions):
            output = args.output.with_name(f"{args.output.stem}-region-{index}.png")
            region_colors.append(screenshot(int(window), width, height, output, left, top))
        animation_frames = None
        if animation_region:
            left, top, width, height = animation_region
            frames = []
            for index in range(4):
                frame = Path(settings.name) / f"animation-{index}.png"
                screenshot(int(window), width, height, frame, left, top)
                frames.append(frame.read_bytes())
                time.sleep(0.13)
            animation_frames = len(set(frames))
            if animation_frames < 2:
                raise RuntimeError("Animation region stayed static across four frames")
        print(json.dumps({"window_appeared_ms": appeared_ms,
                          "idle_cpu_percent_one_core": idle_cpu,
                          "resident_bytes": rss, "rendered_colors": colors,
                          "screenshot": str(args.output), "theme": args.theme, "screen": args.screen,
                          "width": args.width, "height": args.height,
                          "distinct_animation_frames": animation_frames,
                          "paint_region_colors": region_colors,
                          "workload": "external bus" if args.bus else "in-process demo",
                          "scope": "window appearance is not first usable frame; host/build/GPU affect all values"}, indent=2))
        if process.poll() is not None:
            raise RuntimeError("Native process exited during smoke test")
    finally:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        settings.cleanup()


if __name__ == "__main__":
    main()
