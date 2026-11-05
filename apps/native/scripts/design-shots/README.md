# Native design shots

One command captures the whole native (GPUI) app as it looks today: about 74
PNGs covering every page, panel, prompt and settings section, in Dark plus
Light, Nord, Catppuccin Latte and Tokyo Night. It is for before/after
comparisons when the design changes. Nothing is checked in except the rig. The
PNGs go wherever you point `--out`.

```sh
# From the repository root. Builds the debug binaries, starts its own Xvfb.
python3 apps/native/scripts/design-shots/shoot-all.py --out /tmp/native-shots

# The same shots with a redesign switched on (repeatable; applies to every shot).
python3 apps/native/scripts/design-shots/shoot-all.py --out /tmp/native-redesign \
    --setting design_refresh=true

# Iterate on a few shots without rebuilding; list what exists.
python3 apps/native/scripts/design-shots/shoot-all.py --list
python3 apps/native/scripts/design-shots/shoot-all.py --no-build --only settings --only changes --out /tmp/s
```

| Flag | Meaning |
|---|---|
| `--out DIR` | Output folder; files are `NN-slug.png` (`03-conversation-streaming.png`) |
| `--setting KEY=VALUE` | Seeds `native-settings.json` for every shot. VALUE is JSON (`true`, `125`, `"portrait"`); a bare word is a string. Wins over per-shot settings |
| `--theme NAME` | Force one palette for every shot |
| `--only TEXT` | Only shots whose slug contains TEXT (repeatable) |
| `--scale N` | `GPUI_X11_SCALE_FACTOR`; default 2 (HiDPI, 3200x2000 for the 1600x1000 window). `--scale 1` for 1:1 |
| `--width/--height` | Logical window size, default 1600x1000 (tall shots use 1000x1600) |
| `--no-build`, `--release`, `--binary`, `--harness` | Reuse or pick binaries |
| `--keep-temp` | Keep each shot's temp HOME and `app.log` |

Requirements (Debian/Ubuntu): the native build dependencies from
`apps/native/README.md`, plus `xvfb xdotool imagemagick mesa-vulkan-drivers
libvulkan1`. No GPU is needed: lavapipe (llvmpipe) renders. If `DISPLAY` is set
the rig uses it; otherwise (or with `DESIGN_SHOTS_XVFB=1`) it starts its own
Xvfb sized for the largest shot.

Timing on a 4-core container: first debug build about 10 minutes; a full
74-shot run about 13 minutes at scale 2 (each shot is a fresh app launch,
4 s settle plus its steps).

## How it works

- Each shot launches a fresh `wks-native` with `HOME` and `XDG_CONFIG_HOME`
  pointed at a temp folder, writing `workspacer/native-theme.json` (theme) and
  `workspacer/native-settings.json` (settings) there first. Your real
  preferences and credentials are never read or touched.
- Data comes from `native-harness serve` (the same in-process fixture `--demo`
  uses, `src/harness.rs`), started per shot on a free port with
  `--rich-transcript --catalog --turns 0`. That tells one story: a developer
  supervising about 30 distinct sessions across workspacer (and its native
  client), hub-rs, infra and notes. Two need them (the active session's
  `cargo test` approval and a release checklist's question), three are mid-turn,
  five are idle and the rest ended over five days, each with its own activity
  time. The active conversation is one run of Grep, Read, Edit, Write, Bash,
  Skill and spawn calls with results, three subagents (one stalled: no
  heartbeat for 7 minutes), fleet wakes and a failed background test run.
  Counts are kept consistent: the hosts queue's running items are the working
  sessions, both agents are installed (Codex usage is measured), and a
  `--sessions 0` first run has no pending scratch, jobs or history. The
  catalog fleet's table is `LIVE` in `src/harness/catalog.rs`. Variants add
  `--pending-questions`, `--plan-and-limit` (whose usage report agrees that the
  5-hour window is spent) and `--all-clear` (nothing waits on the user). Shot
  01 is the stock `wks-native --demo` (100 sessions with distinct generated
  titles and a plain transcript; it disables session creation). No agents,
  models or network are used.
- Input is real X input through `xdotool` (vim `g <letter>` jumps, Ctrl+K,
  clicks). Capture is ImageMagick `import` of the window; a shot that paints
  fewer than 16 colors fails.
- Clicks use logical coordinates from the `LANDMARKS` table in
  `shoot-all.py`, measured on the baseline layout. A redesign that moves a
  control (the sidebar rail toggle, title-pill chips, approval buttons, the
  settings category rail) only needs that table updated. Keyboard-driven shots
  need nothing.
- The pointer rests in the window's bottom-right corner before capture, so no
  hover state or tooltip is left behind. Settings → About is captured before
  the app's first update check (8 s after launch), because offline that check
  shows a network error a real user would not see.

Known fixture limitations, not app bugs: Settings → Remote shows "Unknown
fixture method" (the fixture does not implement the Tailscale status call), and
the file viewer shows the fixture's fixed source text for every path.

To add a shot, add one `add(...)` line in `shots()`: a slug, a description,
steps (`key`, `keys`, `type`, `click`, `move`, `sleep`, `settings_section`),
and optionally `theme=`, `settings={...}`, `fixture=`, `width=`/`height=`,
`capture_after=`. Numbering follows list order, so keep new shots near their
neighbours.

## Gating a redesign on a boolean setting

Native settings are device-local and live in one serde struct,
`Settings` in `apps/native/src/navigation.rs`, saved as
`$XDG_CONFIG_HOME/workspacer/native-settings.json` (`%APPDATA%` on Windows;
the theme is separate, in `native-theme.json`, `src/appearance.rs`). The struct
is `#[serde(default)]`, so a new field is backward compatible: old files load
it from `Default`. `main.rs` loads the file once (`Settings::load`) and hands it
to `Workspace::configure_settings` (`src/ui/navigation.rs`), which stores it as
`self.settings`. Every screen is an `impl Workspace` render method, so the
value is available everywhere as `self.settings.<field>`. Toggles write it back
with `self.save_settings(cx)`. `--setting` in this rig (and in
`scripts/smoke.py`) simply pre-writes that JSON file.

Recommended key: **`design_refresh`** (a boolean, default `false`). Steps:

1. `src/navigation.rs`: add to `Settings`
   ```rust
   /// The refreshed visual design (work in progress); off keeps the current look.
   pub design_refresh: bool,
   ```
   and `design_refresh: false,` in `impl Default for Settings`. Extend the
   `clock_preference_is_backwards_compatible_and_round_trips` test so `{}`
   loads it as `false` and a `true` round-trips.
2. `src/ui/settings.rs`: add an `Entry` in the `S::Appearance` group, modelled
   on `reduce-motion` / `tool-rail`:
   ```rust
   entries.push(Entry::new(
       S::Appearance,
       "design-refresh",
       "New design",
       "Try the refreshed look. Saved on this device.",
       "design new look refresh redesign preview beta style",
       Layout::Row,
       Switch::new("settings-design-refresh")
           .checked(self.settings.design_refresh)
           .tooltip("New design")
           .on_click(cx.listener(|this, checked, window, cx| {
               this.settings.design_refresh = *checked;
               this.save_settings(cx);
               window.refresh();
           })),
   ));
   ```
   `window.refresh()` repaints every view at once (Window layout does the same).
3. Read it at render time in `Workspace` code: `if self.settings.design_refresh
   { ... }`, or `.when(self.settings.design_refresh, |d| ...)`. Colors come from
   `let p = self.appearance.palette();` (about 200 call sites); for a palette
   change, branch in one place, for example a `Workspace::palette()` helper
   that returns `self.appearance.palette()` or a refreshed variant, and switch
   call sites to it as they are redesigned. Shared building blocks
   (`page_view`, `page_header`, `card`, `notice_line` in `src/ui/chrome.rs`)
   are methods or helpers called from `Workspace`, so pass the flag or the
   palette through them.
4. Code outside `Workspace` does not see `self.settings`: gpui-component
   widgets are themed by `ui::configure_theme(appearance, ...)` (`src/ui.rs`,
   called at startup and by `set_appearance`), and the terminal reads the
   `TerminalPalette` GPUI global. If the redesign must reach those, mirror the
   flag into a small GPUI global (`cx.set_global(DesignRefresh(bool))`, as
   `TerminalPalette` does) in `configure_settings` and in the toggle, and
   re-run `configure_theme` when it changes.
5. Capture: `shoot-all.py --setting design_refresh=true --out ...` and compare
   with a baseline run of the same commit.

## Component gallery (r5-system)

`system-gallery-<theme>` shots (one per built-in theme) launch the app with
`--gallery`, which opens `Screen::Gallery` (debug builds only) in a taller
window (`GALLERY_HEIGHT`). A shot can pass extra app arguments with its
`app_args` key. The gallery renders every component of
`docs/design-system.md` in every state:

    python3 apps/native/scripts/design-shots/shoot-all.py --no-build \
        --setting design_v2=true --only system-gallery --out /tmp/gallery
