#!/usr/bin/env python3
"""Full-app design screenshot rig for the native (GPUI) client.

Builds (or reuses) wks-native and native-harness, then captures every major
screen of the app against isolated fixtures: no credentials, agents or model
calls. Each shot launches a fresh window with its own XDG_CONFIG_HOME, so a
shot's theme and settings never leak into the next one or into your real
preferences. See README.md in this folder.

    python3 apps/native/scripts/design-shots/shoot-all.py --out /tmp/shots
    python3 apps/native/scripts/design-shots/shoot-all.py --out /tmp/new \
        --setting design_refresh=true          # repeatable, applied to every shot
    python3 apps/native/scripts/design-shots/shoot-all.py --only settings --list
"""
import argparse
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
NATIVE = HERE.parent.parent

# Fixture flavors. "rich" is the believable workspace: real-looking projects,
# jobs, scratch and history (--catalog) with a conversation full of tool
# calls, subagents, background tasks and an approval (--rich-transcript).
# "demo" is the stock `wks-native --demo` (100 synthetic sessions).
FIXTURES = {
    "rich": ["--rich-transcript", "--catalog", "--turns", "120"],
    "questions": ["--rich-transcript", "--catalog", "--turns", "120", "--pending-questions"],
    "plan": ["--rich-transcript", "--catalog", "--turns", "120", "--plan-and-limit"],
    # No sessions yet: the first-run welcome (catalog projects to pick from).
    "empty": ["--catalog", "--sessions", "0"],
    "demo": None,
}

SETTINGS_SECTIONS = ["Appearance", "Typography", "Workspace", "Remote",
                     "Agents", "Chat", "Keyboard", "About"]
# area-pages: controls design v2 moved (used when --setting design_v2=true).
LANDMARKS_V2 = {
    "END_SESSION": lambda shot: (1324, 634),        # Session details "End session..."
}
# area-pages: design v2's rail.
SETTINGS_SECTIONS_V2 = ["Appearance", "Typography", "Workspace", "Remote",
                        "Agents", "Keyboard", "About"]


def nav(letter):
    """Normal mode, then the vim `g <letter>` jump (see bind_keys in ui.rs)."""
    return [("key", "Escape"), ("keys", "g", letter), ("sleep", 1.5)]


def send(text="Polish the session header and re-run the UI tests."):
    # Pauses let the composer take focus and lay out the draft before Send.
    return [("key", "ctrl+l"), ("sleep", 0.8), ("type", text), ("sleep", 0.8), ("key", "ctrl+Return")]


# A shot: slug, fixture, steps, plus optional theme / settings / size / wait.
# Coordinates in steps are logical (unscaled) window pixels. The default
# window is 1600x1000 logical, wide layout.
def shots():
    s = []
    add = lambda slug, desc, steps=(), **kw: s.append(dict(slug=slug, desc=desc, steps=list(steps), **kw))

    # Conversation and its surroundings.
    add("demo-conversation", "Stock `--demo` workload: 100 synthetic sessions, plain transcript", fixture="demo")
    add("conversation", "Rich conversation at rest: work cards, markdown, approval dock, tool rail")
    add("conversation-streaming", "Reply streaming in after sending a message (Bus fan-out session)",
        [("click", "SIDEBAR_ROW_BUS"), ("sleep", 1.5)] + send(), capture_after=1.0)
    add("conversation-scrolled", "Earlier in the transcript: worker/supervisor cards, subagents",
        [("key", "Escape"), ("keys", "ctrl+u", "ctrl+u", "ctrl+u")])
    add("conversation-light", "Rich conversation, Light theme", theme="light")
    add("sidebar-rail", "Sidebar collapsed to its icon rail", [("click", "RAIL_TOGGLE")])
    add("session-search", "Ctrl+K session search popover from the rail",
        [("click", "RAIL_TOGGLE"), ("key", "ctrl+k"), ("sleep", 0.8), ("type", "review")])
    add("tall-layout", "Tall window: rail sidebar, portrait layout", width=1000, height=1600)
    add("tall-file-viewer", "Tall window with a file preview docked below the chat",
        [("click", "TALL_README_LINK"), ("sleep", 2)], width=1000, height=1600)
    add("subagent-preview", "Provider-native subagent's read-only conversation preview",
        [("click", "SIDEBAR_ROW_CHILD"), ("sleep", 1.5)])
    add("background-tasks", "Background tasks panel (dev server log, deploy, failed tests)",
        [("move", "TITLE_PILL"), ("click", "TITLE_TASKS_BADGE"), ("sleep", 2)])
    add("approval-details", "Bash permission prompt with its details expanded",
        [("click", "APPROVAL_DETAILS")])
    add("question-prompt", "Pending AskUserQuestion picker (three questions)", fixture="questions")
    add("plan-and-limit", "ExitPlanMode plan approval plus spent 5-hour usage window", fixture="plan")
    add("task-log", "Background task log: the dev server's output",
        [("move", "TITLE_PILL"), ("click", "TITLE_TASKS_BADGE"), ("sleep", 2), ("click", "TASK_ROW_DEV"), ("sleep", 2)])
    add("deny-with-note", "Approval dock: Deny with note", [("click", "APPROVAL_DENY_NOTE"), ("sleep", 0.5),
        ("type", "Run only the session tests, not the whole suite.")])
    add("file-viewer", "File viewer with README.md open from a transcript link",
        [("click", "README_LINK"), ("sleep", 2)])
    add("terminal", "Session terminal panel", [("key", "Escape"), ("key", "ctrl+grave"), ("sleep", 2)])
    add("new-session", "New agent form with a task typed",
        [("key", "Escape"), ("key", "ctrl+n"), ("sleep", 3), ("click", "NEW_TASK_FIELD"),
         ("type", "Profile sidebar scrolling with 1,000 sessions and fix any dropped frames.", 150)])
    # Pages.
    add("projects", "Projects: project groups and folders", nav("p"))
    add("history", "Session history grouped by project", nav("h"))
    add("jobs", "Scheduled jobs: approval, runs, paused", nav("j"))
    add("scratch", "Scratch files with pending items", [("key", "Escape"), ("design_keys", ("g", "n"), ("g", "x")), ("sleep", 1.5)])
    add("hosts", "Hosts: resources and work queues", nav("o"))
    add("changes", "Changes: diff review with stage/commit panel", nav("d"))
    add("setup", "Agent setup (first-run provider checks)", nav("a"))
    add("session-details", "Session details: name, agent, actions, context", nav("e"))
    add("end-session-confirm", "End session confirmation", nav("e") + [("click", "END_SESSION")])
    add("model-picker", "Change model, effort and access", nav("m"))
    add("model-dropdown", "Change model with the model list open", nav("m") + [("click", "MODEL_SELECT")])
    for index, section in enumerate(SETTINGS_SECTIONS):
        # About: capture before the first update check (8 s after launch);
        # offline it would show a network error no real user sees.
        fast = dict(settle=2.0, capture_after=0.8) if section == "About" else {}
        add(f"settings-{section.lower()}", f"Settings: {section}",
            nav("s") + [("settings_section", index)], **fast)
    # Other palettes.
    add("settings-appearance-light", "Settings: Appearance, Light theme", nav("s"), theme="light")
    add("projects-light", "Projects, Light theme", nav("p"), theme="light")
    add("changes-light", "Changes, Light theme", nav("d"), theme="light")
    add("conversation-nord", "Rich conversation, Nord", theme="nord")
    add("conversation-catppuccin-latte", "Rich conversation, Catppuccin Latte", theme="catppuccin-latte")
    add("settings-tokyo-night", "Settings: Appearance, Tokyo Night", nav("s"), theme="tokyo-night")
    # Shell & navigation (design v2 surfaces; v1 shows its own equivalents).
    add("title-hover", "Title capsule hovered: model chip, access, background work", [("move", "TITLE_PILL")])
    add("model-popover", "Model, effort and access from the title's model chip",
        [("move", "TITLE_PILL"), ("click", "TITLE_MODEL"), ("sleep", 1.5)])
    add("shortcuts", "Keyboard cheat sheet (? in Normal mode)", [("key", "Escape"), ("key", "question"), ("sleep", 0.8)])
    add("sidebar-compact", "Compact session list (28px rows)", settings={"sidebar_compact": True})
    add("sidebar-row-hover", "A session row's actions on hover", [("move", "SIDEBAR_ROW_BUS")])
    add("rail-hover-card", "Collapsed rail: a session's hover card",
        [("click", "RAIL_TOGGLE"), ("move", "RAIL_FIRST_MARK"), ("sleep", 1.5)])
    # area-pages: design v2 page states (run with --setting design_v2=true).
    add("hosts-details", "Hosts: this machine's details open (design v2)",
        nav("o") + [("click", "HOSTS_DETAILS")])
    add("jobs-overflow", "Jobs: a job's overflow menu open (design v2)",
        nav("j") + [("click", "JOB_OVERFLOW")])
    add("settings-remote-light", "Settings: Remote, Light theme", nav("s") + [("settings_section", 3)],
        theme="light")
    add("projects-nord", "Projects, Nord", nav("p"), theme="nord")
    add("settings-about-offline", "Settings: About after an offline update check",
        nav("s") + [("settings_section", 7), ("sleep", 8)])
    add("new-session-nord", "New session form, Nord", [("key", "Escape"), ("key", "ctrl+n"), ("sleep", 2)],
        theme="nord")
    # area-transcript: the transcript itself, further back and at each reading width.
    add("transcript-earlier", "Transcript further back: tool rows, work cards, user turns",
        [("wheel", 3, 1000, 400)])
    add("transcript-start", "Transcript start: the first turns",
        [("key", "Escape"), ("keys", *(["ctrl+u"] * 12))])
    add("transcript-fleet", "Transcript: supervisor and fleet wakes, subagents",
        [("wheel", 2, 1000, 400)])
    add("transcript-wide", "Rich conversation at the wide reading width",
        settings={"reading_width": "wide"})
    add("transcript-full", "Earlier transcript at the full reading width",
        [("key", "Escape"), ("keys", "ctrl+u", "ctrl+u", "ctrl+u")], settings={"reading_width": "full"})
    # area-panels: design v2 opens Changes as a dock beside the chat (`g d`,
    # the tool rail); its expand button opens the full page.
    add("changes-docked", "Changes docked beside the conversation (v2; v1 shows the page)", nav("d"))
    add("changes-docked-split", "Docked Changes, side by side", nav("d"), settings={"review_split": True})
    add("changes-docked-light", "Docked Changes, Light theme", nav("d"), theme="light")
    add("changes-page", "Changes full page from the dock's expand button (v2)",
        nav("d") + [("click", "REVIEW_DOCK_EXPAND"), ("sleep", 1.5)])
    add("changes-discard", "Docked Changes: a file's overflow menu",
        nav("d") + [("click", "REVIEW_FILE_MORE"), ("sleep", 0.5)])
    # area-composer: first run and the composer's menus (appended so v1
    # baseline numbering stays stable).
    add("welcome", "First run: no sessions yet, the welcome with setup checklist", fixture="empty")
    add("slash-menu", "Composer `/` menu over the draft", [("key", "ctrl+l"), ("sleep", 0.8), ("type", "/")])
    # Integration: v2 folds Chat into Appearance; its reading width found by search.
    add("settings-reading-width", "Settings search: the transcript's reading width",
        nav("s") + [("click", 1360, 76), ("sleep", 0.5), ("type", "reading width"), ("sleep", 0.8)])
    # r2-transcript: the turn's tool calls as event rows ("Worked … · n tools").
    add("transcript-tools", "Transcript: a turn's tool calls as one-line event rows",
        [("wheel", 6, 1000, 400)])
    return s


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def wait_port(port, timeout=20):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            socket.create_connection(("127.0.0.1", port), timeout=0.5).close()
            return
        except OSError:
            time.sleep(0.1)
    raise RuntimeError(f"fixture did not listen on {port}")


def start_xvfb(width, height):
    for number in range(91, 140):
        if not Path(f"/tmp/.X11-unix/X{number}").exists() and not Path(f"/tmp/.X{number}-lock").exists():
            break
    else:
        raise RuntimeError("no free X display number")
    proc = subprocess.Popen(["Xvfb", f":{number}", "-screen", "0", f"{width}x{height}x24",
                             "-nolisten", "tcp"], stderr=subprocess.DEVNULL)
    deadline = time.monotonic() + 10
    while not Path(f"/tmp/.X11-unix/X{number}").exists():
        if time.monotonic() > deadline or proc.poll() is not None:
            raise RuntimeError("Xvfb did not start")
        time.sleep(0.1)
    return proc, f":{number}"


def parse_setting(text):
    key, sep, value = text.partition("=")
    if not sep or not key:
        raise argparse.ArgumentTypeError("--setting needs KEY=VALUE")
    try:
        return key, json.loads(value)
    except ValueError:
        return key, value  # bare word: a string setting


class Rig:
    def __init__(self, args):
        self.args = args
        self.scale = args.scale
        profile = "release" if args.release else "debug"
        self.app = (args.binary or NATIVE / "target" / profile / "wks-native").resolve()
        self.harness = (args.harness or NATIVE / "target" / profile / "native-harness").resolve()

    def px(self, value):
        return str(int(round(value * self.scale)))

    def xdo(self, *argv, check=True):
        return subprocess.run(["xdotool", *map(str, argv)], check=check, timeout=10,
                              capture_output=True, text=True)

    def park(self, width, height):
        """Rest the pointer in the window's bottom-right corner, which holds no
        control in any layout, so no hover state or tooltip lands in the
        capture. (Leaving the window does not dismiss GPUI tooltips.)"""
        self.xdo("mousemove", int(self.px(width)) - 3, int(self.px(height)) - 3)
        time.sleep(0.2)

    def design_v2(self, shot):
        """The shot runs with design_v2 on (run() sets shot["v2"] from its settings)."""
        return bool(shot.get("v2"))

    def run_steps(self, window, steps, shot):
        for step in steps:
            kind, *rest = step
            if kind == "key":
                self.xdo("key", *rest)
                time.sleep(0.3)
            elif kind == "design_keys":  # (v1 keys, v2 keys): design v2 moved Scratch to g x
                self.xdo("key", "--delay", "120", *(rest[1] if shot.get("v2") else rest[0]))
                time.sleep(0.3)
            elif kind == "keys":
                self.xdo("key", "--delay", "120", *rest)
                time.sleep(0.3)
            elif kind == "type":
                self.xdo("type", "--delay", str(rest[1] if len(rest) > 1 else 30), rest[0])
                time.sleep(0.3)
            elif kind == "sleep":
                time.sleep(rest[0])
            elif kind in ("click", "move"):
                landmarks = LANDMARKS
                if len(rest) != 2 and self.design_v2(shot) and rest[0] in LANDMARKS_V2:
                    landmarks = LANDMARKS_V2  # area-pages: v2 moved this control
                x, y = rest if len(rest) == 2 else landmarks[rest[0]](shot)
                self.xdo("mousemove", "--window", window, self.px(x), self.px(y))
                # Hover first: the title capsule springs open under the pointer.
                time.sleep(0.8)
                if kind == "click":
                    self.xdo("mousedown", "1")
                    time.sleep(0.1)
                    self.xdo("mouseup", "1")
                    time.sleep(0.8)
                    self.park(shot["width"], shot["height"])
            elif kind == "settings_section":
                if self.design_v2(shot):
                    # area-pages: v2's rail folds Chat into Appearance.
                    name = SETTINGS_SECTIONS[rest[0]]
                    index = SETTINGS_SECTIONS_V2.index(name if name in SETTINGS_SECTIONS_V2 else "Appearance")
                    x, y = LANDMARKS["settings_section_v2"](shot, index)
                else:
                    x, y = LANDMARKS["settings_section"](shot, rest[0])
                self.xdo("mousemove", "--window", window, self.px(x), self.px(y))
                self.xdo("click", "1")
                time.sleep(1.0)
                self.park(shot["width"], shot["height"])
            elif kind == "wheel":  # area-transcript: scroll the chat up by n notches
                notches, x, y = rest
                self.xdo("mousemove", "--window", window, self.px(x), self.px(y))
                time.sleep(0.3)
                self.xdo("click", "--repeat", notches, "--delay", "40", "4")
                time.sleep(1.0)
                self.park(shot.get("width", self.args.width), shot.get("height", self.args.height))
            elif kind == "snap":  # debugging: capture an intermediate frame
                path = Path(self.args.out) / f"_snap-{shot['slug']}-{rest[0]}.png"
                subprocess.run(["import", "-silent", "-window", window, f"png:{path}"], check=True)
            else:
                raise ValueError(f"unknown step {kind}")

    def shoot(self, index, shot):
        args = self.args
        width = shot.get("width", args.width)
        height = shot.get("height", args.height)
        output = Path(args.out) / f"{index:02d}-{shot['slug']}.png"
        home = Path(tempfile.mkdtemp(prefix="wks-design-shot-"))
        procs = []
        try:
            config = home / "config" / "workspacer"
            config.mkdir(parents=True)
            (config / "native-theme.json").write_text(json.dumps(args.theme or shot.get("theme", "dark")))
            settings = dict(shot.get("settings", {}))
            settings.update(args.settings)  # command-line --setting wins
            if settings:
                (config / "native-settings.json").write_text(json.dumps(settings))
            env = dict(os.environ, HOME=str(home), XDG_CONFIG_HOME=str(home / "config"),
                       GPUI_X11_SCALE_FACTOR=str(self.scale))
            env.pop("WAYLAND_DISPLAY", None)
            env.pop("WKS_HUB_BUS", None)
            env.pop("HUB_TOKEN", None)
            fixture = FIXTURES[shot.get("fixture", "rich")]
            log = open(home / "app.log", "w")
            if fixture is None:
                command = [str(self.app), "--demo"]
            else:
                port = free_port()
                procs.append(subprocess.Popen(
                    [str(self.harness), "serve", "--bind", f"127.0.0.1:{port}", *fixture],
                    stdout=subprocess.DEVNULL, stderr=log, env=env))
                wait_port(port)
                command = [str(self.app), "--bus", f"ws://127.0.0.1:{port}/bus"]
            app = subprocess.Popen(command, stdout=log, stderr=log, env=env)
            procs.append(app)
            window = subprocess.check_output(
                ["xdotool", "search", "--sync", "--onlyvisible", "--pid", str(app.pid)],
                timeout=60, text=True).split()[0]
            self.xdo("windowmove", window, 0, 0)
            self.xdo("windowsize", window, self.px(width), self.px(height))
            self.xdo("windowfocus", "--sync", window, check=False)
            time.sleep(shot.get("settle", args.settle))
            # Park the pointer in the bottom-right corner so no hover state shows.
            self.park(width, height)
            self.run_steps(window, shot["steps"], dict(shot, width=width, height=height,
                                                         v2=bool(settings.get("design_v2"))))
            time.sleep(shot.get("capture_after", args.wait))
            if app.poll() is not None:
                raise RuntimeError(f"app exited ({app.returncode}); see {home / 'app.log'}")
            subprocess.run(["import", "-silent", "-window", window, f"png:{output}"],
                           check=True, timeout=60)
            colors = int(subprocess.check_output(["identify", "-format", "%k", str(output)], text=True))
            if colors < 16:
                raise RuntimeError(f"only {colors} colors painted; inspect {output}")
            return output, colors
        finally:
            for proc in reversed(procs):
                proc.terminate()
                try:
                    proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    proc.kill()
                    proc.wait()
            if args.keep_temp:
                print(f"   kept {home}")
            else:
                shutil.rmtree(home, ignore_errors=True)


# Window landmarks, in logical pixels, measured from the baseline build.
# Kept in one place so a redesign that moves a control only edits here.
LANDMARKS = {
    "RAIL_TOGGLE": lambda shot: (286, 35),          # sidebar header, rightmost icon
    # Design v2 groups the list (Needs you, then Idle): Bus fan-out sits higher.
    "SIDEBAR_ROW_BUS": lambda shot: (110, 411 if shot.get("v2") else 468),
    "SIDEBAR_ROW_CHILD": lambda shot: (110, 212),   # "Inspect transcript parsing"
    # The title pill grows when hovered (context meter appears), so hover it
    # first; these two are the chips' positions once it has grown.
    "TITLE_PILL": lambda shot: (900, 30),
    "TITLE_MODEL": lambda shot: (880 if shot.get("v2") else 1005, 30),  # "Sonnet" chip
    "TITLE_TASKS_BADGE": lambda shot: (1160 if shot.get("v2") else 1078, 30),  # background work
    # v1: the approval card above the composer; v2: the strip on the composer.
    "APPROVAL_DETAILS": lambda shot: (1289, 833) if shot["v2"] else (1361, 749),
    "APPROVAL_DENY_NOTE": lambda shot: (1032, 869) if shot["v2"] else (910, 795),
    "RAIL_FIRST_MARK": lambda shot: (35, 205 if shot.get("v2") else 219),  # first session in the rail
    "README_LINK": lambda shot: (651, 629) if shot["v2"] else (591, 497),  # README.md link, last assistant message
    "TALL_README_LINK": lambda shot: (228, 1229) if shot["v2"] else (169, 1097),  # same link, 1000x1600 tall layout
    "TASK_ROW_DEV": lambda shot: (1200, 85),        # "npm run dev" in the tasks panel
    "MODEL_SELECT": lambda shot: (955, 207),        # Change model page, model select
    "NEW_TASK_FIELD": lambda shot: (1150, 560),     # New agent form, Task box
    "END_SESSION": lambda shot: (1337, 679),        # Session details "End session..."
    # area-pages (design v2 layouts)
    "HOSTS_DETAILS": lambda shot: (1345, 211),      # Hosts: first machine's "Details"
    "JOB_OVERFLOW": lambda shot: (1366, 420),       # Jobs: first scheduled job's "..."
    # area-panels: the v2 Changes dock's header and first file header.
    "REVIEW_DOCK_EXPAND": lambda shot: (1535, 30),  # dock header "Open full page"
    "REVIEW_FILE_MORE": lambda shot: (1563, 74),    # first file's "More actions"
    # Settings category rail; rows are ~37.7 px apart starting at Appearance.
    "settings_section": lambda shot, index: (570, 114 + round(index * 37.7)),
    # area-pages: the design v2 rail (Chat folds into Appearance).
    "settings_section_v2": lambda shot, index: (476, 158 + round(index * 34.4)),
}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--out", default="design-shots", help="output directory (created)")
    parser.add_argument("--setting", dest="settings", action="append", type=parse_setting, default=[],
                        metavar="KEY=VALUE", help="seed native-settings.json for every shot; VALUE is JSON "
                        "(true, 125, \"x\") or a bare string; repeatable, wins over per-shot settings")
    parser.add_argument("--theme", help="force one palette for every shot (dark, light, nord, ...)")
    parser.add_argument("--only", action="append", default=[], help="only shots whose slug contains this; repeatable")
    parser.add_argument("--list", action="store_true", help="list shots and exit")
    parser.add_argument("--width", type=int, default=1600)
    parser.add_argument("--height", type=int, default=1000)
    parser.add_argument("--scale", type=float, default=2.0,
                        help="GPUI_X11_SCALE_FACTOR (default 2: HiDPI, 3200x2000 PNGs for a "
                        "1600x1000 window); 1 for 1:1 pixels")
    parser.add_argument("--settle", type=float, default=4.0, help="seconds after the window appears")
    parser.add_argument("--wait", type=float, default=1.5, help="seconds after the last step")
    parser.add_argument("--release", action="store_true", help="build/use target/release")
    parser.add_argument("--no-build", action="store_true", help="reuse existing binaries")
    parser.add_argument("--binary", type=Path, help="wks-native to use (implies --no-build)")
    parser.add_argument("--harness", type=Path, help="native-harness to use")
    parser.add_argument("--keep-temp", action="store_true", help="keep each shot's temp HOME and app.log")
    args = parser.parse_args()
    args.settings = dict(args.settings)

    selected = [(i, s) for i, s in enumerate(shots(), 1)
                if not args.only or any(o in s["slug"] for o in args.only)]
    if args.list:
        for index, shot in selected:
            print(f"{index:02d}-{shot['slug']}: {shot['desc']}")
        return
    for tool in ("xdotool", "import", "identify"):
        if not shutil.which(tool):
            sys.exit(f"{tool} is required (apt-get install xdotool imagemagick)")

    started = time.monotonic()
    if not (args.no_build or args.binary):
        cmd = ["cargo", "build", "--locked", "--bin", "wks-native", "--bin", "native-harness"]
        if args.release:
            cmd.append("--release")
        print("+", " ".join(cmd), flush=True)
        subprocess.run(cmd, cwd=NATIVE, check=True)
    rig = Rig(args)
    for path in (rig.app, rig.harness):
        if not path.exists():
            sys.exit(f"missing {path}; build it or drop --no-build")

    xvfb = None
    if not os.environ.get("DISPLAY") or os.environ.get("DESIGN_SHOTS_XVFB"):
        w = int(max(args.width, *(s.get("width", 0) for _, s in selected)) * args.scale) + 64
        h = int(max(args.height, *(s.get("height", 0) for _, s in selected)) * args.scale) + 64
        xvfb, display = start_xvfb(w, h)
        os.environ["DISPLAY"] = display
        print(f"Xvfb on {display} ({w}x{h})")
    Path(args.out).mkdir(parents=True, exist_ok=True)
    failures = []
    try:
        for index, shot in selected:
            t0 = time.monotonic()
            try:
                output, colors = rig.shoot(index, shot)
                print(f"{output.name:44s} {colors:6d} colors {time.monotonic() - t0:5.1f}s  {shot['desc']}", flush=True)
            except Exception as error:  # keep going; report at the end
                failures.append((shot["slug"], error))
                print(f"FAILED {shot['slug']}: {error}", flush=True)
    finally:
        if xvfb:
            xvfb.terminate()
    print(f"{len(selected) - len(failures)}/{len(selected)} shots in {time.monotonic() - started:.0f}s -> {args.out}")
    if failures:
        sys.exit(1)


if __name__ == "__main__":
    main()
