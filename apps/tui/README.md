# wks-tui

A terminal client for [workspacer](../README.md). It talks directly to the
**claudemon** daemon's REST + SSE API, so it can monitor every live Claude agent
and drive it from a terminal with no Electron window open. Only `claudemon`
needs to be running.

> Why not the hub bus? The hub's `agents.*` / `claude.*` capabilities (what the
> `/remote` web client calls) are registered by the **Electron main process** —
> they don't exist when the desktop app isn't running. claudemon, by contrast,
> exposes the full surface (list, transcript, approve/answer/message/signal,
> spawn, `/events`) over loopback HTTP, so a standalone client uses it directly.

This is the early TUI: agent monitoring, control, and spawning. Raw-PTY terminal
panes, the workspace/tab model, library, analytics and the command palette are
planned in later phases (see the repo's TUI plan).

## What it does today

- **Command palette** — `Ctrl-K` opens a fuzzy launcher: start a new agent, open
  a terminal, jump to the Dashboard, jump to any running agent, or insert a
  **library** item into the focused agent. Type to filter, `↑`/`↓` to move,
  `enter` to run.
- **Library** — reusable prompts / skills / agents loaded from
  `~/.config/workspacer/library/*.md`, `~/.claude/skills/*/SKILL.md`, and
  `~/.claude/agents/*.md`. From the palette each item offers **Run in new
  agent** (spawn an agent — pick cwd/profile — seeded with the prompt once it's
  ready) or **Insert** (paste it into the focused agent). Both use bracketed
  paste so multi-line prompts stay intact and aren't auto-submitted.
- **Dashboard** — a pinned row at the top of the sidebar showing mission-control
  overview: agent count, how many need you / working / idle, total cost, and a
  compact roster. The TUI is a dashboard over **every** Claude claudemon sees,
  not just ones launched here.
- **Tabs per agent** — open an agent and it has a tab bar. The first tab is the
  Claude session; `T` opens a **terminal tab** (a real shell, spawned via
  claudemon, so it also shows in the system-wide list). `[` / `]` switch tabs,
  `w` closes one.
- **Terminal path** — by default the chat view renders the agent's **raw PTY**
  (vt100), exactly like the Electron pane's terminal toggle: you see Claude's
  real terminal, live. Press `i`/`enter` to **attach** (your keystrokes go
  straight to Claude — numbers, `y`/`n`, Esc, arrows, Ctrl-C), `Ctrl-]` to
  detach. `t` toggles to the parsed transcript ("GUI path") with approve/answer
  buttons. The pane resizes claudemon's PTY so Claude reflows to fit. Terminals
  are kept **warm in the background** — leave an agent and come back and it's
  already up to date, no blank re-attach. Sessions claudemon only *observes*
  (started outside the TUI — there's no PTY to stream) automatically show their
  **transcript** instead, so the TUI works as a dashboard over every Claude
  running on the machine, not just ones you launched here.
- **Spawn agents** — `c` opens a modal to pick a working directory and a Claude
  profile; spawns a fresh session via claudemon. Model and skip-permissions ride
  along in the profile's args, as in the desktop app.
- **Agent sidebar** — every live session, sorted so the ones needing you float
  to the top, with state badge, model, context % and cost. These prefer Claude's
  authoritative **statusLine** (streamed live from claudemon — same numbers
  Claude shows itself), falling back to transcript-derived usage when it hasn't
  arrived yet. The Dashboard also shows 5h/7d rate-limit windows when Claude
  reports them (Pro/Max).
- **Chat view** — the selected agent's transcript (text + tool calls), live as
  it streams. Long runs of consecutive tool calls (e.g. during a workflow)
  collapse into one compact `N tool calls · …` line.
- **Review pane** — `R` opens a git review of the agent's work tree (backed by
  claudemon's git API, like the desktop Review pane): branch + changed files on
  the left, the selected file's colourised unified diff on the right. Stage /
  unstage / commit / push without leaving the terminal.
- **Approvals** — `y` / `n` / `a` to approve, deny, or always-approve a pending
  permission prompt.
- **Questions** — `1`–`9` to pick an `AskUserQuestion` option, or type a free
  answer.
- **Messaging** — send a prompt to an agent that's at an input prompt.
- **Signals** — `x` interrupt (SIGINT), `X` stop (SIGTERM).
- **Reconnect** — survives claudemon restarting; the header shows connection state.

## Keys (vim-first)

**Sidebar** (row 0 is the Dashboard, then agents)

| Key | Action |
|-----|--------|
| `j` / `k` | move down / up |
| `g` / `G` | first (Dashboard) / last |
| `enter` / `l` | open the selected agent (its tabs) |
| `c` | new agent (spawn modal) |
| `Ctrl-K` | command palette (works anywhere) |
| `m` | jump to the next agent needing attention |
| `y` / `n` / `a` | approve / deny / always (if it has a pending approval) |
| `1`–`9` | answer a pending question |
| `r` | refresh |
| `q` | quit |

**Agent view — tabs**

| Key | Action |
|-----|--------|
| `[` / `]` (or Shift-Tab/Tab) | previous / next tab |
| `T` | new terminal tab (shell in the agent's cwd) |
| `w` | close the active tab (closing the Claude tab leaves the agent) |

**Agent view — terminal path (default)**

| Key | Action |
|-----|--------|
| `i` / `enter` | attach — keystrokes go to the terminal |
| `Ctrl-]` | detach (back to navigation) |
| `t` | toggle to the transcript (GUI) view (Claude tabs only) |
| `x` / `X` | interrupt (SIGINT) / stop (SIGTERM) |
| `c` | new agent · `esc`/`h` back · `q` quit |

While **attached**, every key (incl. Esc, arrows, Ctrl-C) goes to the agent;
only `Ctrl-]` is intercepted.

**Chat — transcript path (`t`)**

| Key | Action |
|-----|--------|
| `t` | toggle back to the terminal |
| `i` | insert mode (compose a message / answer) · `esc` back to normal · `enter` send |
| `j` / `k` | scroll the transcript |
| `y` / `n` / `a` | approve / deny / always |
| `1`–`9` | answer a pending question |
| `x` / `X` | interrupt / stop |
| `esc`/`h` back · `q` quit |

**Spawn modal** (`c`)

| Key | Action |
|-----|--------|
| (type) | edit the working directory |
| `tab` | complete the path (longest common prefix; lists candidates when ambiguous) |
| `↑` / `↓` | cycle the Claude profile |
| `enter` | spawn |
| `esc` | cancel |

**Review pane** (`R` — from the sidebar or an open agent)

| Key | Action |
|-----|--------|
| `j` / `k` | select previous / next changed file |
| `J` / `K` | scroll the diff by a line |
| `Ctrl-d` / `Ctrl-u` (or PageDn/Up) | scroll the diff by a chunk |
| `t` | toggle staged ⇄ unstaged diff |
| `s` / `u` | stage / unstage the selected file |
| `a` | stage everything |
| `c` | commit (type a message, `enter` to commit) |
| `P` | push |
| `r` | refresh status |
| `esc` / `h` / `q` | close the review |

## Configuration

Optional, read from `~/.config/workspacer/tui.json` (the same config dir the
desktop app and Claude profiles use). Everything has a sane default; a missing
or malformed file is ignored (with a warning) rather than blocking startup. Open
the in-app help overlay with `?` to see the active bindings and theme list.

```json
{
  "theme": "nord",
  "colors": { "accent": "#88c0d0", "warn": "yellow" },
  "keys": {
    "list":             { "x": "quit", "q": "none" },
    "agent_transcript": { "ctrl+r": "refresh" },
    "global":           { "f1": "help" }
  }
}
```

**Theme** — `theme` picks a built-in preset (`default`, `nord`, `gruvbox`,
`ansi`); `ansi` uses your terminal's own 16-color palette. `colors` overrides
individual roles (`accent`, `ok`, `warn`, `bad`, `dim`, `fg`, `selection_bg`) on
top of the preset. A color is `#rrggbb`, a bare `rrggbb`, an ANSI name (`cyan`,
`darkgray`…), or a 256-color index.

**Keys** — `keys` remaps bindings per context. Contexts: `global` (checked
everywhere), `list` (sidebar/dashboard), `agent_terminal` (raw PTY / shell
tabs), `agent_transcript` (parsed transcript). A chord is a `key` with optional
`ctrl+` / `alt+` / `shift+` modifiers (e.g. `ctrl+k`, `shift+tab`, `G`, `space`,
`enter`, `esc`). Map a chord to an action name (press `?` for the full list —
`quit`, `select_next`, `attach`, `approve`, `toggle_transcript`, …), or to
`none` to remove a default. The composer, spawn field, palette query, and the
positional `1`–`9` answer keys are not remappable.

## Running

```sh
cargo run --release
```

### Daemons

`wks-tui` is a client of the `claudemon` daemon. When pointed at a **local**
daemon (the default), on startup it launches claudemon if it isn't already
listening — and stops it again on exit. So running it standalone Just Works; if
the Electron app (or a claudemon you started by hand) is already up, that one is
reused and left running.

This needs the claudemon binary built in-tree (`cargo build --release` in
`claudemon/`). Override its location with `WKS_CLAUDEMON_BIN`, or pass
`--no-spawn` to connect only to a daemon started elsewhere. Auto-spawn is
skipped when `--claudemon-url` points at a non-loopback host.

By default it talks to `http://127.0.0.1:7891`. Override with:

```sh
wks-tui --claudemon-url http://my-host:7891
```

If you see a persistent **"reconnecting…"** in the header, claudemon isn't
reachable — either its binary is missing (check the startup diagnostics) or you
passed `--no-spawn` with nothing running.
