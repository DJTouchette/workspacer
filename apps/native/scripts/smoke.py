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
import zlib


class XImage(C.Structure):
    _fields_ = [(name, C.c_int) for name in ("width", "height", "xoffset", "format")] + [
        ("data", C.c_void_p)
    ] + [(name, C.c_int) for name in (
        "byte_order", "bitmap_unit", "bitmap_bit_order", "bitmap_pad", "depth",
        "bytes_per_line", "bits_per_pixel"
    )]


def screenshot(window, width, height, output):
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
    image = x.XGetImage(display, window, 0, 0, width, height, 0xFFFFFFFF, 2)
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
    args = parser.parse_args()
    command = [str(args.binary.resolve())]
    command += ["--bus", args.bus] if args.bus else ["--demo"]
    started = time.monotonic()
    process = subprocess.Popen(command, stdout=subprocess.DEVNULL)
    try:
        window = subprocess.check_output(
            ["xdotool", "search", "--sync", "--onlyvisible", "--pid", str(process.pid)],
            timeout=20, text=True).splitlines()[0]
        appeared_ms = (time.monotonic() - started) * 1000
        def drive(*commands):
            subprocess.run(["xdotool", *commands], check=True, timeout=5)
        drive("windowfocus", window, "windowsize", window, "1000", "700")
        time.sleep(args.settle_seconds)
        before, _ = process_sample(process.pid)
        idle_start = time.monotonic()
        time.sleep(args.sample_seconds)
        after, rss = process_sample(process.pid)
        idle_cpu = 100 * (after - before) / (time.monotonic() - idle_start)
        drive("key", "ctrl+l")
        drive("type", "Native window smoke test")
        drive("key", "ctrl+Return")
        time.sleep(1)
        colors = screenshot(int(window), 1000, 700, args.output)
        print(json.dumps({"window_appeared_ms": appeared_ms,
                          "idle_cpu_percent_one_core": idle_cpu,
                          "resident_bytes": rss, "rendered_colors": colors,
                          "screenshot": str(args.output),
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


if __name__ == "__main__":
    main()
