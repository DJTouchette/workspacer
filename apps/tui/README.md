# wks-tui

A terminal client for [workspacer](../README.md). It monitors every live Claude
agent and drives it from a terminal with no Electron window open.

By default it's a thin client of the **hub bus**: driving (message / approve /
answer / signal / spawn), the agent list (`agent.snapshot` / `agent.statusline`),
and the live terminal (`pty.bytes`) all flow through the hub's headless **brain**
provider — the same capabilities the desktop app and `/remote` web client use, so
every client mirrors one source of truth. For a loopback bus it auto-spawns the
hub + brain (and claudemon), so it Just Works standalone.

Pass `--direct` to bypass the bus and talk to **claudemon**'s REST + SSE API
directly (the original standalone path) — useful when the hub binary isn't built
or you want one fewer process. The TUI also falls back to this automatically if a
loopback bus isn't reachable.

The TUI is a working bus client for monitoring, control, spawning, raw PTY
terminal views, tabs/splits, library insertion, review, notes, and attention
handling. It is still intentionally narrower than the desktop app around
analytics and rich browser/editor workflows. See
[`docs/features.md`](../../docs/features.md) for the repo-wide maturity catalog.

## What it does today

- **Command palette** — `Ctrl-K` opens a fuzzy launcher over several sources:
  start a new agent, open a terminal, run any `:` **command** (`vsplit`, `pin`,
  `review`, …), jump to any running agent (findable by its **cwd**, not just its
  name), or insert a **library** item into the focused agent. Type to filter
  (matches label + hint), `↑`/`↓` to move, `enter` to run.
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
- **Splits (windows)** — tile several agents at once with vim's `Ctrl-w` prefix:
  `Ctrl-w v` splits the content into columns, `Ctrl-w s` into rows (each split
  brings the next agent into view). `Ctrl-w w` / `Ctrl-w h`/`l` move focus,
  `Ctrl-w o` keeps only the focused pane, `Ctrl-w q`/`c` closes it. The **focused**
  pane is fully interactive (attach, transcript, tabs, approve); the others show
  that agent's **live terminal read-only** (a waiting agent gets an amber border
  so it still draws your eye). Capped at 4 tiles. The `Ctrl-w` menu also appears
  in the which-key popup.
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
- **Leader menu (which-key)** — press the leader (`space` by default; set
  `"leader"` in `tui.json`) to pop a menu of the next keys and what they do:
  `p` palette, `a` new agent, `t` terminal, `n` notes, `r` rename, `g` review,
  `m` jump-to-attention, `S` respawn, `?` help, `q` quit. Bindings are multi-key
  sequences — remap them in `tui.json` with whitespace-separated chords (e.g.
  `"<leader> x": "quit"`). `esc` cancels a half-typed sequence.
- **Vim counts** — prefix a motion with a number: `3j` / `3k` move three rows,
  `5G` jumps to agent 5, and a count multiplies transcript scroll. Digits become
  a count only when no question is pending (otherwise `1`-`9` still answer it);
  the pending count shows in the footer.
- **Command line (`:`)** — vim's ex line for verbs that don't need a key:
  `:q` quit, `:vsplit`/`:split`/`:only`/`:close` windows, `:spawn` / `:term`,
  `:notes` / `:review` / `:pin`, `:rename <name>`, `:filter <query>`,
  `:ls` (dashboard), `:help`. `enter` runs, `esc` cancels.
- **Content search (`<leader>/`)** — grep every agent's transcript at once.
  Opening it indexes each session's conversation in the background (the title
  shows progress); type to filter matching lines live, and `enter` jumps to that
  agent's transcript. Also `:search` / `:grep`. (Searches transcript text;
  file-body search would need a claudemon grep endpoint and isn't wired yet.)
- **Filter the sidebar** — `/` opens a live fuzzy filter (subsequence match over
  name / cwd / state). Type to narrow the list as you go; `enter` keeps the
  filter and returns to `j`/`k` navigation, `esc` clears it. The filter is just a
  view — the open agent, splits, pins and fleet totals all still see the full set.
- **Harpoon (pinned agents) + jumps** — pin the agents you're juggling and
  teleport between them. `<leader>h` pins/unpins the current agent (a `⚓N` badge
  shows its slot in the sidebar); `<leader>1`…`<leader>9` jump straight to a
  pinned slot. `Ctrl-^` toggles to the **alternate** agent (the one you were just
  on), and `Ctrl-o` / `<leader>o` step **back** through your jump history with
  `<leader>i` forward. Pins are **persisted by cwd** (`tui-pins.json`), so they
  survive a restart — each pin re-resolves to whatever agent is live in that
  directory (same stable identity as names/notes).
- **Agent sidebar** — every live session in a stable order (rows stay put across
  polls; new sessions append at the end), with state badge, model, context % and
  cost. Stopped sessions you never saw alive this run are hidden by default —
  claudemon replays up to 100 prior sessions as `stopped` on restart, and that
  history would otherwise flood the list. Ones that stop *while you're watching*
  stay (so you can respawn them); `<leader>x` toggles the full history on/off and
  the title shows how many are hidden (`agents (3 · +12 stopped)`). TUI-spawned
  **shell tabs** are also kept out of the sidebar (and the dashboard / pickers) —
  they live inside their agent's tab bar, not as standalone rows. These prefer
  Claude's
  authoritative **statusLine** (streamed live from claudemon — same numbers
  Claude shows itself), falling back to transcript-derived usage when it hasn't
  arrived yet. The Dashboard also shows 5h/7d rate-limit windows when Claude
  reports them (Pro/Max).
- **Chat view** — the selected agent's parsed conversation (from claudemon's
  `/conversation`), live as it streams: text, tool calls **with their output**
  (a dimmed `↳` snippet, red on error), and inline diffs' summaries. Long runs of
  consecutive tool calls (e.g. during a workflow) collapse into one compact
  `N tool calls · …` line. An open agent's tab bar shows a git inspector
  (`⎇ branch ±changed`).
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
- **Rename / respawn** — `e` gives an agent a custom name (per project / cwd,
  persisted to `~/.config/workspacer/tui-names.json`); `S` respawns a stopped
  agent with a fresh Claude in its working directory.
- **Notes** — `N` opens a per-project markdown scratchpad (`i` to edit, `esc` to
  save), persisted per cwd to `~/.config/workspacer/tui-notes.json`.
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
| `e` | rename (a custom per-project name, persisted) |
| `S` | respawn a stopped agent (fresh Claude in its cwd) |
| `R` | open the git review pane |
| `N` | open the notes scratchpad |
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

On startup the TUI launches whatever it needs that isn't already listening, on
loopback, and stops it again on exit (a pre-existing one — the Electron app, or a
daemon you started by hand — is reused and left running):

- **claudemon** always (the engine).
- in the default **bus** mode, the **hub + brain** too (`make build-hub` puts the
  `hub`/`brain` binaries in-tree; override the hub with `WKS_HUB_BIN`). Point at a
  remote hub with `--bus ws://host:7895/bus`; auth with `--bus-token` / `HUB_TOKEN`.

So running it standalone Just Works. With `--direct` only claudemon is needed.

claudemon must be built in-tree (`cargo build --release` in `claudemon/`); override
its location with `WKS_CLAUDEMON_BIN`, or `--no-spawn` to connect only to daemons
started elsewhere. Auto-spawn is skipped for a non-loopback host.

By default it talks to `http://127.0.0.1:7891`. Override with:

```sh
wks-tui --claudemon-url http://my-host:7891
```

If you see a persistent **"reconnecting…"** in the header, claudemon isn't
reachable — either its binary is missing (check the startup diagnostics) or you
passed `--no-spawn` with nothing running.

## Testing

The TUI has unit coverage across key parsing/dispatch, app state transitions,
bus calls, claudemon response parsing, terminal attachment state, review flow
state, themes, config, and rendering helpers. Run it from this package with:

```sh
cargo test
```

Ignored live tests in `src/claudemon.rs` exercise a real daemon when you need to
check PTY/list behavior against a running claudemon.
