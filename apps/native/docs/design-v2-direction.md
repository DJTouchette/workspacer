# Design v2: direction

Everything here ships behind `design_v2` (Settings → Appearance → "New design (preview)", `--design-v2`).
With the flag off, the app must render exactly as before. Tokens, primitives and the "no literals"
rule are covered in `design-v2.md`. This file covers *what* v2 should look and behave like.

North star: **calm by default, dense on demand, attention first.** Workspacer already has the deepest
supervision features of any agent tool: decision cards, jobs, hosts and limit handoff. The redesign
stops showing all of that at once. Every screen should answer "what needs me?" within a second.
After that, power is one deliberate step away.

## 1. Colour discipline (biggest single lever)
- **One accent** (blue) for primary actions, links and focus, and nothing else.
- **Four status tokens, used only for status:**
  - attention = amber (needs you)
  - running = blue, animated
  - success = green (terminal success only)
  - error = red
- Idle, Ready and Ended use neutral tertiary text. **Purple is retired.**
- **Provider/model identity is neutral.** Provider glyphs and model names use tertiary grey everywhere. Brand colour (Claude clay, etc.) appears only on the single model chip in the composer/title. Amber is the only warm hue in the sidebar.
- A resting screen shows **≤ 3 hues** (neutral, accent and at most one status colour).
- Per-theme contrast floors must hold in all 8 themes, enforced by tests:
  - text ≥ 4.5:1
  - primary button vs surface ≥ 3:1
  - on-accent text ≥ 4.5:1
  - Nord's primary must not look disabled.
  - Light needs real surface separation (1px borders on cards), darker amber/orange text tokens, and a neutral selected-row fill.
- Code blocks sit on the card surface with a subtle border, not darker than the canvas.

## 2. Systems (one component per role)
- **StatusBadge:** one shape (dot + label) and one colour map, used everywhere (sidebar, projects, jobs, hosts, background tasks, transcript cards, rail).
  - Vocabulary: **Needs you · Working · Idle · Done · Failed** (plus Paused / Queued where real).
  - Never show the brand mark as a spinner. Use a fixed-box 12px spinner or pulse.
- **Buttons:** variants Primary / Secondary / Ghost / Danger-ghost, **2 heights (28 / 36)**, **2 radii** (8 for controls; full for chips and badges).
  - **One primary per region.** While a decision card is open, the composer's send demotes to secondary.
  - Destructive actions (Remove, Discard, Stop) go into an overflow menu or ghost-danger, with confirmation or undo.
- **PageHeader:**
  - optional back link
  - title (22/600)
  - one-line description
  - right-aligned actions
  - fixed top offset
  - shared content max-width
  - Caps eyebrows only at page-section level; card eyebrows use sentence case in the status colour, and change with state (e.g. "Decision made", never "Decision needed" + "Resolved").
- **Type scale:** 22 / 16 / 14 / 13 / 12 (11 only for kbd/meta), weights 400/500/600. Every `text_size(px(..))` in v2 paths reads tokens.
- **Avatars:** monogram tiles with a deterministic muted hue and 8px radius. No emoji.
- **Overlays:** shared popover/menu/dialog shells. Floating chrome (title capsule, Jump to latest) gets a fade mask so content never shows through or clips.
- **Formatting:**
  - Humanised durations: over 48h shows as `34d 3h`; stale timers show "Lost heartbeat" in warning.
  - Timestamps without seconds.
  - Long values truncate with ellipsis and tooltip; never overlap.

## 3. Shell & navigation
- **Attention-first sidebar:**
  - A pinned **"Needs you (n)"** group, oldest-waiting first, then **Working**, then **Idle**, then **Ended** (collapsed by default).
  - A key to jump to the next session that needs you (`g n`), and the inbox badge goes there too.
  - **Rows:**
    - Comfortable: ~44px, two lines.
    - Compact density option: 28px single line (status, title, project muted, relative time).
    - Model shown only when it differs from the default.
    - Per-row action icons on hover/focus only.
    - Children get a 1px tree guide.
- **Footer:** one ≤ 44px row (account/setup menu, compact usage, Jobs, Settings), all labelled or all icon-with-tooltip, consistently. Setup prompts collapse into one dismissible "Finish setup (n)" item, and providers that aren't installed get no usage meter. Usage reads "5-hour limit · 42% · resets 15:40" in tooltip; red only at ≥ 95%.
- **Vim/modal chrome hidden by default.** The mode line appears only when modal keys are enabled (Settings → Keyboard) and only while in a non-default mode. Key hints move into tooltips and a `?` cheat sheet.
- **Title capsule:** title + neutral model chip + always-visible access mode. Counts get words ("3 subagents"). Either a solid docked header or a fade mask beneath it.
- **Tool rail:** docked to the content edge or the window edge, grouped with separators, with labelled tooltips including shortcuts.
- **Rail (collapsed sidebar):** the status ring is the dominant signal (amber = needs you, animated = working). Hover card shows title, status and age. The rail is hidden when the sidebar is expanded.
- **Wordmark:** "Workspacer" next to the mark in the expanded sidebar.
- **Vocabulary:** **Session** = a conversation with an agent, **Agent** = who runs it, **Subagent** = only inside a session. Plain words replace ops jargon on everyday surfaces:
  - gate → "Accepting work / Paused: low memory"
  - wake → "Why this was raised"
  - fleet worker finished → "2 agents finished"
  - PSI → tooltip only
  - Internal IDs go behind "Copy ID".

## 4. Conversation
- **Reading width:** comfortable / wide / full. On wide windows, reclaimed width goes to a docked panel rather than empty gutters.
- **Tool calls:** compact single-line rows by default (`● Read src/launch.rs · 42 lines`, expand on click). Resolved supervisor/fleet events collapse to one line. Older turns fold ("37 earlier steps").
- **One card anatomy:** icon, title, status badge, meta line, actions right.
- User messages hug their content (max ~70%), with no "You" label.
- Markdown table text columns are left-aligned.
- **Decision cards** (already best-in-class, keep their structure):
  - A pending approval becomes a dense strip attached to the composer: `Bash · cargo test   [1] Allow  [2] Always allow in project  [3] Deny  [4] Deny with note  · Details`.
  - Details shows cwd, host, requesting agent and reason, not raw JSON.
  - Stacked decisions show "1 of 2", and plan+limit should offer "Approve and continue with Codex".
- **Composer:** a single primary whose label reflects state ("Send" when idle; "Steer" while working, with explanatory tooltip). Queue / Send-now go into a split-button menu. The hint row moves into tooltips. "Waiting…" says what it waits on.
- **Model / effort / access:** a popover anchored to the model chip, with type-ahead. The Access segmented control gets an explicit "Unknown" state.

## 5. Panels & pages
- **Changes** opens as a resizable right dock beside the transcript (full page is still available):
  - all files stacked, with per-file +/−
  - collapsed unchanged regions
  - git header lines hidden
  - unified/split toggle
  - `]c` / `[c` hunk navigation
  - status badges (Modified/New/Staged) instead of bare letters
  - Discard in overflow
- **Hosts:** a one-line summary per machine first ("studio · ready · 3 slots free"). Metrics and queue sit behind Details. Fixed status column, no label collisions.
- **Jobs:** keep the approval-card pattern. Add a "New job" button that opens a session prefilled for the agent. Remove goes to overflow. The run-history sparkline gets a tooltip.
- **Projects:** folders nest under their project, with a single "Ungrouped" section. "pending" becomes "1 needs you" and is a link.
- **Settings:** fluid grids, no dead gutters; tiny sections fold together (Chat into Appearance or similar).
- **New session:** equal-width host cards, "Best fit" as a separate Auto-place toggle, closed gates muted.
- **First run / empty state:** a centred welcome in the conversation area with:
  - wordmark and a one-line value statement
  - the agent setup checklist inline
  - a project picker
  - the composer with 3 starter suggestions

## Quality bar
Every change sits behind `design_v2`. It reads tokens, keeps existing `debug_selector`s (add new ones for new elements), keeps tests green, and is checked with the screenshot rig (`scripts/design-shots/shoot-all.py --setting design_v2=true`) in dark, light and at least one alternate theme before handing off.
