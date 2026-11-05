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
  Approval chips: 1 Allow = primary; 2 Always allow = secondary; 3 Deny = destructive; 4 Deny with note = tertiary (still clearly clickable). *(r5-integration: superseded by the R5.4 DecisionCard. The numbers now follow reading order, 1 Allow, 2 Deny, 3 Always allow, 4 Deny with note; see `design-system.md`.)* Approve/Reject, Refresh/Check again and the rest all map to these roles.
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

---

# Round 3 addendum

Round-2 council average: 6.9 (newcomer 6.7, systems 6.6, power user 7.2, UX lead 7.2, visual 6.8). The reviews converge on one diagnosis: **every state is drawn several ways, everywhere, at the same volume.** Round 3 is about subtraction and structure. Reviews: `scratchpad/council/r2/*.md`.

## R3.1 One signal per state (status language)
- **Lexicon** (the only words used for state, everywhere): *Working · Needs you · Not responding · Failed · Idle · Done*. Retire "running" for agents, "stalled", "Lost heartbeat", "Check", "Delegating/Delegated" as state words.
- **Glyph per state** (8px, same everywhere: sidebar, rail, capsule, palette, history, machines, children): Working = ring with rotating arc (static arc under reduce-motion); Needs you = filled amber dot; Not responding = amber/red hollow clock or ring-with-gap; Failed = red ✕ dot; Idle = **no glyph** (age text only); Done = muted check. Machines/jobs map onto these, plus neutral `Paused` (pause glyph) and `Offline` (dash). "Unknown" is never amber.
- **Amber = blocked on you, nothing else.** Scratch counts, "decision needed" on other sessions, jobs proposed by agents: neutral badges with counts, except the item that is actually waiting on *this user now*.
- **Red = something broke, always named, always actionable.** Title capsule: "3 running" neutral text; a separate red chip "cargo test failed" (names the item) that opens it and can be acknowledged (then gone). Never paint healthy state red.
- One "attention" treatment: a 3px inset bar clipped by the container radius plus a neutral border. Sidebar Needs-you row = bar + title + reason line; verb chip appears on hover/selection only.

## R3.2 Components: three button tiers, enforced
- Primary (filled accent), Secondary (1px border ghost, same height/radius), Tertiary (text). Destructive = Secondary shape with danger text. No bare red text buttons, no grey filled chips as buttons.
- Approval card: Allow = Primary; Always allow (shows exact scope, e.g. "cargo test in apps/native") = Secondary; Deny = Secondary split button whose menu holds "Deny with note…". Number keys are faint trailing hints from one keycap token, never tinted.
- **While a decision is pending, the decision card occupies the composer slot** (the composer collapses to a one-line "Reply instead…" affordance). No duplicate placeholder text, no second input.
- Composer: one primary button in a fixed place whose label changes (Send / Queue / Stop · Esc); secondary send modes live in its dropdown. Context shows as "Context 42%" text, coloured only ≥80%.
- Every primary-button pair passes 4.5:1 in all themes (Nord/Tokyo Night Allow included); add a test across all themes.

## R3.3 Structure, not floating
- Title is a real header strip (36–40px) with the transcript scrolling beneath; no content under it.
- Session tools become a labelled, badged toolbar attached to structure (header or window edge): Changes (+5 −2), Files, Terminal (●), Tasks (3), History, Details, Model. 16px icons, 28px hit areas, tooltips with shortcuts.
- "Jump to latest" anchors 12px above the composer/decision stack, solid surface + shadow, a ≥32px scrim behind it, shows "↓ N new".

## R3.4 Transcript: one column, one rhythm
- One content width for prose, tool rows, cards and tables (fluid, capped); durations sit inline with the result ("· 2 matches · 6s"), not on a far right edge.
- A completed work block collapses to one line by default ("Worked 5m 40s · 6 tools ›"); the agent's reply and the "Changed 2 files · Review" card lead each turn. Short tool output (≤5 lines) shows inline, dimmed mono, under its row; failures inline in red with exit code.
- Cross-session notices are one-line events ("Release checklist for 0.9 needs you · Go"), never full cards, never internal ids.
- 8px spacing rhythm: 32px above headings, 12px below, 16px between paragraphs.
- Failures always offer recovery: task log gets Retry and "Ask agent to fix"; a not-responding subagent offers "View output · Nudge · Stop".
- Change counts are labelled by scope ("This turn +5 −2" vs "Branch +17 −3"); Review from a turn filters to that turn.

## R3.5 Plain language & honest defaults
- Jargon pass: no "hub", "ctx", "spawn", "p2", "connection", "title capsule", "demo-NNNN" ids, "Normal ?" footer, tag soup in Scratch. Scratch reads as plain language (or nests under Projects).
- Model popover always lists known models per provider; "Custom id…" last. Access always shows the effective mode selected.
- Usage: "Claude 42% of 5-hour limit used · resets 2h 13m", amber ≥80%, red at 100%.
- Settings descriptions: user outcomes, ≤2 lines, no broken strings.

## R3.6 Density & layout
- One sidebar row template for all groups (title + time line 1; project/status line 2; compact drops line 2 everywhere); title truncates last.
- Secondary pages use the width: Jobs/Machines/History as dense left-aligned tables; offline machines collapse to one muted line; no 60%-empty cards.
- Avatars: curated 8-hue ramp at equal lightness per theme, AA text; no second ring colour; status as corner badge using the R3.1 glyph.
- Light theme tuned on its own (no peach borders, AA metadata greys); three text greys per theme.

## R3.7 Fleet view (new)
- A "Fleet" view: a grid of live session tiles (2×2 / 3×2, responsive). Each tile: title, R3.1 status, last event line, and the pending ask with its own approve/answer keys. Click or Enter opens the session; keyboard moves focus between tiles. Reachable from nav, palette and a shortcut. v2 only.

---

# Round 4 addendum

Round 3 council (all five reviewers, average 7.3, up from 6.9) agreed on one root cause: **the same session tells a different story on different surfaces.** Round 4 fixes truth first, then calm, then consistency.

## R4.1 One source of truth for status
- Every surface (header strip, composer, sidebar row, History row, Fleet tile, palette, title chip) reads a session's state from ONE derivation with ONE priority order: Failed (unacknowledged, actionable) > Needs you > Not responding > Working > Paused/Offline > Idle > Done. No surface computes its own.
- A parent whose subagents are working is **Working** everywhere (sidebar, History, Fleet badge, Fleet grouping). Fleet and sidebar counts must match.
- A failure chip clears (or mutes to history) once the same command later succeeds; it always names its task ("cargo test failed · Restore sessions").
- A session that is streaming is never "Not responding" (fixture must update activity while streaming).
- Change counts: one scope, one label. Header and Changes panel show the same number, labelled ("Branch +17 −3").

## R4.2 "Needs you" is complete
- Anything blocked on the user counts: tool approvals, questions, plan reviews, **job proposals awaiting approval**. Jobs nav gets the needs-you count. "All clear" only when the count is truly zero.

## R4.3 One approval component everywhere
- Conversation, Fleet tile, Jobs proposal and plan review use the same component: full command (wrapped, never truncated next to Approve), folder and host, Allow = Primary, the exact scoped rule for "Always allow …", Deny/Deny with note = Secondary. Details expands to the full request, not a menu.
- Keycap numbers appear only on the focused/top decision, never on every button.

## R4.4 Calm
- Three text levels only: primary, secondary, faint. Row meta is faint and one line; restating defaults is deleted.
- Colour: amber = needs you only (capacity warnings use neutral, then red at a hard limit); red = actionable failure only, one red signal per failure; working = neutral/accent, not a third hue.
- Header toolbar: icon buttons with tooltips + shortcuts, History/Details/Model under one overflow; the session title never truncates while space exists.
- Sidebar: one-line rows by default, compact nav, one selection style, one needs-you style; collapsed-rail avatars use the tuned ramp.
- Bottom padding so no text ever runs under the Jump-to-latest pill or a pinned decision.

## R4.5 One page template
- `page_header`: back affordance (consistent on every page or none), title, one-line intro, right actions in one slot, same left edge on every page including New session. One New-session flow.
- Tables reserve fixed action slots; actions align and never clip.

## R4.6 Plain language and units
- Every percentage says what it is: "Claude · 42% of 5-hour limit used", "Context 42% full".
- "Reply instead…" says the approval stays pending. Nudge, Scratch and Fleet get a one-line explanation where first shown.
- Distinct icons: Fleet ≠ subagent ≠ Agents settings; Details ≠ About ≠ Shortcuts; Retry ≠ Refresh.

## R4.7 Fleet for bursts
- After answering a tile, focus moves to the next blocked tile. "Needs you only" filter. Ctrl+1…9 jumps to sessions (listed in shortcuts). Tiles in one row share a height. Model names use catalog casing.
- Top-level Not responding sessions get Nudge / Stop in the header strip.

## R4.8 Light theme layering
- Light themes separate canvas, sidebar and cards by surface tint and hairlines, not just borders.

---

# Round 5 addendum: make it a system

Round 4 averaged 7.3 (flat). Reviewers who judge workflow went up (7.6, 7.6, ranking us first). Reviewers who judge craft held or dropped (7.1, 7.1, 7.0), and almost every defect they named is the same component drawn two ways. Round 5 stops fixing screens one at a time and builds an **enforced design system**: one written spec, one component per job, a gallery that renders every component in every theme, and tests that fail when a surface styles locally.

## R5.1 The spec: `docs/design-system.md`
Written: [`design-system.md`](design-system.md).
This is the single source of truth, and it supersedes the token notes in `design-v2.md` wherever they disagree. It defines:
- **Foundations:**
  - colour roles
  - three text levels
  - type scale
  - 4/8 spacing
  - radii
  - **elevation**: flat, raised and overlay. An overlay always has a shadow and a scrim.
  - motion
- **Components:**
  - Button (tiers below)
  - IconButton (always a tooltip with its shortcut)
  - NavItem (icon + label)
  - StatusCell (fixed glyph slot, label, optional detail; *every* state has a glyph, including healthy)
  - Toggle (off is unmistakably off in every theme)
  - TextField / Composer (one composer, used by welcome, New session and conversation)
  - Menu (anchored to its trigger)
  - Overlay / Dialog (shadow + scrim)
  - Card / Tile (natural height)
  - Table (fixed status slot, fixed action slot)
  - DecisionCard (approval, question, plan, job proposal)
  - PageShell (one template for every page: same top, same width rule, same header)
  - CountBadge
  - Keycap
- **Patterns:**
  - page anatomy
  - conversation hierarchy (prose > activity > meta; a finished turn folds into one summary row coloured by its *final* outcome)
  - Fleet card anatomy
  - empty, loading and error states

## R5.2 Button rules
- Tiers: **Primary** (one per surface, the forward action), **Secondary**, **Tertiary**, **Destructive** (Stop, Deny, Discard: never brand blue).
- Send is Primary only when it can send. When it can't, it is disabled, never outlined-when-active.
- Every page header's main action uses the same tier and slot.

## R5.3 Enforcement
- A **component gallery** screen (`Screen::Gallery`, harness/dev only) renders every component in every state.
- **Shots:** the gallery is shot in every built-in theme.
- **Contrast test:** every component state (Primary text on fill, Toggle on/off, status glyphs, faint text on every surface) is checked in **every** theme at 4.5:1 for text and 3:1 for non-text. Nord's pale Allow and the light theme's off toggles must fail this test before they are fixed.
- **Design-lint test:** v2 surface modules may not call raw colour, raw px sizes, or `.shadow`/`.border` outside `primitives`/`tokens`. Allowed exceptions are listed in one place with a reason.

## R5.4 Then migrate every surface onto the system
- **Shell:**
  - labelled NavItems with distinct icons (no two folders)
  - badges that never shift icons
  - IconButtons with tooltips
  - PageShell everywhere: New session and Agent setup share the top and width rule of the other pages
- **Conversation:**
  - three-level hierarchy, with space between turns
  - the composer is always a visible bordered field, even while a decision is pending
  - one subagent truth (the header and sidebar must agree on "Not responding")
  - a recovered failure is not red at the turn level
- **Decisions and Fleet:**
  - one DecisionCard, with Allow and Deny side by side and a short "Always allow" whose exact rule and scope are shown
  - grants listed in Session details and revocable
  - keycaps on the focused card only
  - Fleet cards at natural height, questions answerable on the card, job proposals as cards so the counts match
  - a dense row-per-session Fleet table view
- **Overlays:** the palette, shortcuts and menus get elevation + scrim, and menus anchor to their trigger.

---

# Round 6 addendum: close the gaps between the system and the screen

Round 5 averaged 7.3 (7.7, 7.4, 7.4, 7.2, 7.0). The system exists, but the screens don't fully use it yet, and some of its own rules are wrong on screen. These were verified by the manager in r5-final shots 39 and 75:
- The palette's scrim covers only the conversation pane; the sidebar stays bright.
- The light-theme off toggle has a dark knob, so it reads as on.
- Usage meters truncate mid-word.

**Process change:** a QA gate agent now reviews every shot against this addendum and the r5 reviews *before* the council, and fixes what it finds.

## R6.1 Overlays and surfaces
- **Overlays mount at the window root.** The scrim covers the whole window, sidebar included, at 45–55%.
- **Elevation.** Popovers and menus use the raised surface plus a visible shadow, and never sit on top of text they don't own.
- **Anchoring.** A card's menus open inside or below the card; the Deny menu must not cover "Always allow". The queue menu must not cover the timestamp.
- **Toggle.** Off is a light track, a white knob and a hairline. On is an accent track and a white knob. The off state is the system default and is checked visually in every theme.
- **Nord.** Verify the Allow button in shot 42 at 4.5:1. If any surface still draws its own Allow, route it through the button tier.

## R6.2 One status table
- Publish one table in `design-system.md`: state → glyph → colour → label. It covers Working, Needs you, Not responding, Failed, Idle, Ended, Queued/Waiting to start, Unknown, Ready, Done, Paused and Offline.
- **Every state has a distinct glyph.** Idle, Ended, Waiting and Unknown must differ.
- **Separation test.** A test checks that Needs-you amber and fault red stay perceptually distinct in every theme (light theme included).
- **Warning hue.** Add a warning hue for capacity (meters at 80% or more), distinct from both amber and red.
- **Recovered failures.** A recovered failure shows its count in words at the turn level ("1 failed, then passed"), not a green check, and not red.
- **Stalled subagents.** A stalled subagent is a Needs-you item, with Nudge, Stop and View output wherever it appears (Fleet tile, Fleet table, subagent view).

## R6.3 One decision bar
- **One component.** `DecisionCard` is the only approve/deny UI: conversation, Jobs, Fleet tiles and Fleet table. The wording is consistent (approve/deny for proposals follows the same component and tiers). The Primary button is always filled, and keys show on every keyed decision.
- **Deny with note** replaces the buttons. It never stacks a second Allow/Deny row.
- **One accent per card.** Either the amber edge or the focus ring, never both stacked.
- **Destructive commands.** The risk chip is visible, and Allow needs a second confirm (press again, or a held key).
- **Where keys go.** A visible indicator shows where digit keys go: the focused decision, or the composer.
- **Composer while pending.** While a decision is pending, the composer is a single-line bordered field ("Reply instead…") that expands on focus.

## R6.4 Composer and access
- **Access chip.** It sits next to the model chip ("Sonnet · Ask first"). Full access is shown in the warning hue. It applies immediately, with no Apply button.
- **One term.** "Ask first" everywhere (not "Ask to approve").
- **One New-session layout.** It is the welcome screen's centred composer; New session *is* that screen.

## R6.5 Shell density and labels
- **Labelled toolbar.** At wide widths, header tools carry short labels: "Changes +17 −3", "Files", "Terminal", "Tasks 3". They become icon-only below the width token.
- **Sidebar rows.** One line (title + time). The second line appears only for a Needs-you ask.
- **Usage.** A bar plus a number ("Claude 42%"), with the full sentence in the tooltip.
- **Repeated facts.** Facts repeated on every row of Projects and Machines move to group headers.
- **Page templates.** Two only: centred form, and full-width table. "Back to chat" goes from top-level pages.

## R6.6 Conversation
- **Cross-session notices** leave the transcript. They become a slim strip above it, or the sidebar only.
- **Spacing rhythm.** A fixed 8/16/32 rhythm, enforced through tokens.
- **Meta rows.** "Used skill", "Worked" and "delegated" collapse into one inline meta line per turn.
- **Reading width.** The default column widens to use the pane (Comfortable 880, Wide 1100).
- **Change counts.** The turn card, the Changes panel and the dock agree on files, insertions and deletions, or are labelled with different scopes.

## R6.7 Fleet at scale
- The sidebar collapses to the rail on the Fleet page.
- Cards in a row share their height. The decision row comes first.
- Sort by "waiting longest".
- Batch-approve identical requests.
- The selected filter is clearly selected.
