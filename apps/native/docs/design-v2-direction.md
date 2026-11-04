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

---

# Round 2 addendum (after review round 1)

Review round 1 found that each area was styled on its own: the parts are individually good, but
the product reads as "capable but unedited". Round 2 is about **cohesion, honesty of state, and
restraint**. These rules override anything above that conflicts with them.

## R2.1 Status roles: colour AND shape, used only for status
| Role | Meaning | Colour | Shape |
|---|---|---|---|
| attention | blocked on the user (approval, question, plan, failure needing a decision) | amber | filled dot, amber left stripe on its container |
| fault | failed, stalled ("Not responding · 2m"), offline, error | red | filled dot / ring with bar |
| working | actively running | cyan/teal, distinct from accent blue | pulsing ring |
| done | finished successfully | muted green | check or filled dot |
| idle/ended | nothing to do | tertiary grey | hollow dot |

- **Blue (accent) means "interactive" only:** primary buttons, links, focus, selection. It never encodes state.
- **Amber only for attention.** "Unknown", "Not installed", "Plan ready" and usage go neutral unless the user must act.
- **No contradictory pairs.** An agent with no heartbeat is **Stalled** (fault): it moves into Needs you, shows "Not responding · 2m", and offers Check / Stop actions. It is never also "Working".
- **One source for every count.** Sidebar, header, Projects, Hosts and the transcript must agree.

## R2.2 Component contract (enforced; no local styling)
- **Buttons:** exactly four roles everywhere.
  - primary: filled accent, at most one per region
  - secondary: subtle fill + ring
  - tertiary: text/ghost
  - destructive: red text/ghost, filled red only inside a confirm step
  Approval chips: 1 Allow = primary; 2 Always allow = secondary; 3 Deny = destructive; 4 Deny with note = tertiary (still clearly clickable). Approve/Reject, Refresh/Check again and the rest all map to these roles.
- **"Waiting on you" container:** ONE shared container for approval, question, plan and limit.
  - Layout: neutral surface, amber 3px left stripe, amber 35% border, a question-style headline (e.g. "Run `cargo test` in apps/native?"), and a where/why line always visible.
  - Plan + usage limit merge into ONE card: Approve plan = primary; "Approve · run with Codex" = secondary, shown only when a limit applies; "Keep planning" = tertiary.
  - Multi-question uses a stepper: one question at a time, "2/3", number keys, no inner scroll.
  - While anything is pending, the composer dims, its placeholder reflects the block, and its send is not primary.
- **One card style plus one inset style.** Never more than ONE container level in the transcript; inner items use hairline dividers. No 24px icon tiles: bare 14px icons.
- **One event row** for agent and tool events: icon, verb, primary argument, result summary, then status/duration right-aligned. Consecutive events group under "Worked 2m 14s · 18 tools" (expandable).
- **One table style, one section-header style** (sentence case 12/600, no uppercase anywhere), and **one avatar** (rounded-square monogram tile, hash hue at fixed OKLCH lightness/chroma, also in the rail).
- **Type scale:** 12/13/14/15/17/20/24, weights 400/500/600, tabular numerals for times, counts and stats.
- **Shared page grid:** every full page, Settings included, uses the same content width and left edge. Card grids align, with no ragged button rows.
- **Contrast:** meta text ≥ 4.5:1 in all 8 themes; Light amber text ~#B45309; Nord primary ≥ 4.5:1 label.

## R2.3 Restraint: diagnostics and jargon off by default
- **Hidden by default:** tokens, cost, timings, raw API model IDs and heartbeats move to hover/Details or a "Show diagnostics" setting.
- **Model names:** one formatter, human name everywhere ("Sonnet 4.6", "GPT-5.6 Luna"), raw id on hover. The popover offers a real model list plus "Custom…".
- **Vocabulary:** "machine", not hub/host/peer/slot; "Pick a machine for me", not Auto-place; "Send" when idle, "Send now"/"Queue" while working (Steer moves into the dropdown, with an explanation). No "Prefill", "Dispatched" or "Estimated from tools" in default UI.
- **Vim/modal:** OFF by default. When on, the mode shows as a small label inside the sidebar footer, never floating over the list. Help copy is written for mouse users first.
- **Access chip:** hidden when unknown (or a muted "Access: not reported"). "Unknown" is never a selectable option.
- **"New design (preview)":** lives in a Labs section of Settings.
- **Setup:** one state set (Ready / Needs sign-in / Not installed) and one source, with copy matched to the state and a copyable install command.

## R2.4 Shell
- **Needs you is a pure inbox:** only actionable items, each with its reason and an inline action (Allow / Answer / Open). Children collapse into "4 subagents · 1 stalled" unless a child is itself blocked. "All clear" state when empty.
- **Sidebar rows:** Idle/Ended rows are single-line 30px (title 13, project dim, right-aligned relative age in tabular numerals). Two lines only for Needs you and Working. Idle shows the 8 most recent plus "Show N more". Duplicate titles are disambiguated by age/branch.
- **Search:** filters every group, highlights matches, shows a count.
- **Navigation:** one labelled list of global destinations at the top of the sidebar (Codex-style): New session, Projects, Jobs, Machines, Scratch, Settings. The footer holds only account/usage ("Claude 42%" with a meter). No duplicate entry points.
- **Session tools:** attached to the conversation column, not floating at the window edge, with labels or tooltips showing shortcuts and badges where there is state.
- **Header:** session title is primary and never truncated before the project crumb. 32px fade under it. No stray underline.
- **Rail:** Needs you and Working only, as status-ringed avatar tiles, with "+N" overflow.
- **Ctrl+K:** a command palette covering sessions, actions, files and settings, with prefix scopes (`>` actions, `@` sessions), fuzzy matching and recents.

## R2.5 Conversation
- **Fluid transcript:** min 680, max ~1100, grows with the window. Prose is capped at ~80ch; code, tables, diffs and tool output span the column.
- **Tool calls:** first-class one-line rows, visible by default (the fixture and rig must show them).
- **Failures are loud:** a failed background task turns the header chip red ("3 running · 1 failed") and appears in Needs you.

## R2.6 Pages
- **New session:** task-first. A full-width task box at the top, then machine, project, agent and model as chips (Codex pattern).
- **Hosts → Machines:** consistent counts, a single Refresh, aligned project grid.
- **Welcome:** one 24px question as the heading, no stray pills, setup consistent with Setup and the sidebar.

## R2.7 Demo / fixture coherence
- **Believable fixture data:** realistic, distinct session titles (no "· 2 … · 9" clones), activity times so ages render, consistent counts across surfaces, visible tool calls, one intentionally stalled agent shown correctly, and no impossible states.
