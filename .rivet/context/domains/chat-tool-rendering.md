---
title: Chat transcript, tool-call rendering & composer send pipeline
tags: [renderer-ui, chat, tool-calls, composer, diff-cards, providers, skill]
related_paths:
  - "apps/desktop/src/renderer/src/components/claude/*.tsx"
  - "apps/desktop/src/renderer/src/components/claude/SkillCard.tsx"
  - "apps/desktop/src/renderer/src/contexts/SkillInventoryContext.tsx"
  - "apps/desktop/src/renderer/src/components/claude-shared.tsx"
  - "apps/desktop/src/renderer/src/lib/turnChanges.ts"
  - "apps/desktop/src/renderer/src/lib/providerCaps.ts"
  - "apps/desktop/src/renderer/src/lib/slashItems.ts"
  - "apps/desktop/src/renderer/src/lib/modelLabel.ts"
owner: Damien Touchette
last_reviewed: 2026-08-16
---

# Chat transcript, tool-call rendering & composer send pipeline

## Overview
`apps/desktop/src/renderer/src/panes/ClaudePane.tsx` owns the whole conversation timeline: it groups raw `session.conversation` turns into `ConversationMessage` bubbles interleaved with collapsible tool-call cards, freezes per-turn changed-file snapshots, and drives the message-send + composer-pill affordances. Tool-call rendering and diff/read expansion are provider-agnostic on purpose — claude, codex, opencode and pi all funnel through the same `ToolCall` shape and a small set of exhaustive name/shape maps that must be kept in sync when a provider is added.

## Key modules
- `apps/desktop/src/renderer/src/panes/ClaudePane.tsx` — orchestrator: builds `renderedConversation` (turn-grouping, `WorkView` selection, `ChangedFilesCard` lifecycle), owns send/approval/answer/restart handlers, wires the `/` picker via `useLibrary`.
- `apps/desktop/src/renderer/src/components/claude/WorkCard.tsx` — "cards" work-log style: prose summary (`summarizeWork`) + inline diffs when expanded; exports `summarizeWork` (also imported by `ToolTraceCard.tsx`).
- `apps/desktop/src/renderer/src/components/claude/ToolTraceCard.tsx` — "trace" work-log style: waterfall rows on a shared time axis; drop-in prop-compatible with `WorkCard` (`toolCalls`, `subagentByToolId`, `workflowByToolId`, `live`, `isLast`, `cwd`).
- `apps/desktop/src/renderer/src/components/claude/ConversationMessage.tsx` — renders only user/assistant text bubbles; tool calls/diffs never render here (that's WorkCard/ToolTraceCard's job) — memoized on `turn.content` via `parseMarkdownBlocks`.
- `apps/desktop/src/renderer/src/components/claude/ChangedFilesCard.tsx` — end-of-turn file tree from a frozen `TurnChangeSnapshot`; header shows a `~` when `snapshot.gitAvailable` is false.
- `apps/desktop/src/renderer/src/components/claude/DiffView.tsx` — `DiffView` (Edit/MultiEdit old/new-string, `hasDiff`), `PatchDiffView` (unified-patch `diff` string), `ReadView`/`hasRead` (cat -n parsing).
- `apps/desktop/src/renderer/src/lib/turnChanges.ts` — `EDIT_TOOL_NAMES`, `collectEditedFiles`, `patchLineCounts`, `captureTurnSnapshot`/`estimateSnapshot`, module-level LRU cache (`ensureTurnSnapshot`/`getTurnSnapshot`, `PER_SESSION_CAP = 50`).
- `apps/desktop/src/renderer/src/lib/providerCaps.ts` — `PROVIDER_CAPS` per `AgentProvider`, `CLAUDE_STREAM_CAPS` transport override, `capsFor(provider, transport)`, `permissionModeLabel`.
- `apps/desktop/src/renderer/src/components/claude/ComposerControls.tsx` — pill UI consuming `capsFor`; `liveModelSwitch`/`livePermissionSwitch` call `claudeSetModel`/`claudeSetPermissionMode` and fall back to the restart confirm menu on `!res.ok`.
- `apps/desktop/src/renderer/src/components/claude/Composer.tsx` — textarea with Enter-sends/Shift+Enter-newline, `MAX_COMPOSER_HEIGHT = 168` auto-grow cap, `FileChips`, and the `/` slash picker (`filterSlashItems`).
- `apps/desktop/src/renderer/src/lib/slashItems.ts`, `apps/desktop/src/renderer/src/lib/modelLabel.ts`, `apps/desktop/src/renderer/src/components/claude-shared.tsx` — supporting types/helpers (`SlashItem`, model label formatting, shared `claudeColors`/`WorkLogEntry`/`sendApproval`).
- `apps/desktop/src/renderer/src/lib/turnChanges.test.ts` — unit tests for `collectEditedFiles`/`patchLineCounts`.

## Failure modes
- `captureTurnSnapshot` catches `git.status()` failure and falls straight to `estimateSnapshot` (non-repo cwd, deleted worktree); numstat calls are individually `.catch(() => [])`'d so a numstat hiccup doesn't drop the whole card.
- A file the agent committed mid-turn won't `matchStatusFile` in the post-commit `git status`, so its row silently reverts to the tool-input estimate (both `captureTurnSnapshot`'s "not matched" branch and `estimateSnapshot` share this fallback).
- `ensureTurnSnapshot` dedupes concurrent captures per `sessionId:groupKey` via `inFlight`; the LRU eviction (`PER_SESSION_CAP = 50`, insertion-ordered) silently degrades old turns to the estimate fallback — same visible effect as an app restart, so don't assume an old `ChangedFilesCard` reflects real git numbers.
- `closeGroup` in `ClaudePane.tsx` only emits a card when `ambientIdle` is true (not merely "not streaming" — `waiting_approval` is mid-turn), so a turn stuck on approval never gets a premature/incorrect snapshot.
- Live model/permission switches in `ComposerControls.tsx` treat any daemon `!res.ok` (including a 409) uniformly: `setSwitching(null)` and reopen the same menu as a restart confirm with `res.error` as the reason — there's no separate error UI, so a daemon that never returns `ok:false` correctly will silently retry-forever via the 15s `switchTimerRef` timeout instead.
- `Composer.tsx`'s slash picker only activates on a bare `/token` regex (`/^\/(\S*)$/`) with no space yet; typing a space permanently steps the picker aside for that message even if the user backspaces (state resets only via the `slashQuery === null` effect).

## Gotchas
- **Twin edit-tool inventories that must move together**: `turnChanges.ts`'s `EDIT_TOOL_NAMES` (`edit, multiedit, write, notebookedit, apply_patch, patch`, lowercased) and `WorkCard.tsx`'s `EDIT_TOOLS` (`Edit, MultiEdit, NotebookEdit, apply_patch, patch` — note `Write` handled in a separate `else if` branch there, not in the set) are two independently-maintained lists for the same concept. `ToolTraceCard.tsx`'s `categoryOf` and `DiffView.tsx`'s `hasDiff` are a *third* and *fourth* place that classify tool names by provider shape. Adding a new provider/edit-tool name means touching all four (plus `callTargetFile` in `ToolTraceCard.tsx` if the tool is file-shaped) or the UI silently drifts — one surface will show the diff/category, another won't.
- **Codex multi-file shape**: `apply_patch` can carry either a top-level `diff` string (single file, path from `file_path`/`path`) or `changes: [{ path, diff }]` (multi-file) — `collectEditedFiles` and `summarizeWork` both special-case `Array.isArray(tc.input?.changes)` *before* falling through to the single-diff path; `hasDiff`/`DiffView` only handle the single-file `diff` case (rendered via `PatchDiffView`, not `DiffView`) — multi-file `changes` entries are summarized but not individually diff-rendered inline.
- **WorkCard vs ToolTraceCard parity**: `WorkView = config.claude?.workLog === 'trace' ? ToolTraceCard : WorkCard` in `ClaudePane.tsx` is the only place the choice is made; both components independently reimplement the same open/collapse-on-supersede lifecycle (`active = (live || isLast) && !hasOrchestration`, `wasActive` ref pattern) and the same header stats row — a UX fix in one (e.g. collapse timing, failed-count coloring) needs mirroring in the other by hand, there's no shared base component.
- **Snapshot key stability**: `ChangedFilesCard` snapshots are keyed by the turn-group's first assistant turn's *global conversation index* (`group.start`), computed by walking backward past window edges (`while (gs > 0 && conversation[gs-1].role === 'assistant') gs--`) — this only stays stable because the conversation array only ever appends; anything that could reorder or truncate history would silently break snapshot lookups.
- **Restart-preserves-conversation is per-provider copy, not verified behavior**: `pi`'s `restartPreservesConversation: false` is explicitly commented as "unverified, so the copy promises the safer thing" — don't trust it as ground truth for pi session continuity.
- **`CLAUDE_STREAM_CAPS` is a manual override, not a derived variant**: `capsFor` special-cases `provider === 'claude' && transport === 'stream'` to return a hand-maintained copy of most of `PROVIDER_CAPS.claude` (same `permissionModes`/`effort` levels by reference, but restated `modelSwitch`/`permissionSwitch`/`restartPreservesConversation`) — a change to claude's PTY caps object won't automatically propagate to the stream variant except where it explicitly reuses `PROVIDER_CAPS.claude.permissionModes`.
- **Estimate line counts are approximate by construction**: `patchLineCounts` counts raw `+`/`-` prefixed lines including only skipping `+++`/`---`/`***` headers — it does not distinguish real content changes from patch-context artifacts, and MultiEdit's `edits` array estimate sums whole-string line counts rather than an actual diff, so `ChangedFilesCard` totals for the non-git fallback are directional, not exact.

## Hand-authored notes (2026-07-29) — working timer & where Stop lives

- The streaming row is now `BrandSpinner + WorkingTimer` ("Working for 1m 35s"); the inline
  cancel button (`.wks-stop-btn`, deleted from App.css) MOVED to the composer as
  `.wks-composer-stop`, driven by `Composer`'s new `working` + `onStop` props. It sits
  BESIDE send, never replacing it — a message typed mid-turn is queued for the next turn,
  so swapping send for stop would remove a real action. `aria-label` changed from "Cancel"
  to "Stop" (three pane tests assert on it).
- `workStartedAt` in `ClaudePane` comes from the newest USER TURN's timestamp (read through
  a ref, so a follow-up queued mid-run can't restart the clock), which is what makes the
  count survive a remount or an attach to a session already mid-turn.
- **Order matters in the reset effect**: `if (isStreaming) … else if (ambientIdle) …`. The
  optimistic bridge right after a send is `isStreaming` while `ambientState` is STILL
  `'idle'`, so the idle branch first left the label blank for the entire settle window
  (caught by ClaudePaneOptimisticLoading.test.tsx). The start is cleared on idle rather
  than on `!isStreaming` so a turn parked on an approval keeps its clock.
- `WorkingTimer` owns its own 1s interval: ticking from `ClaudePane` would repaint the whole
  transcript every second for one changing string.
- `fmtDuration` (`components/claude/agentUtils.ts`) gained an hours branch (`1h02m`); it is
  shared with the subagent/workflow rows, which previously rendered "83m00s" past an hour (now "1h 23m").

## Hand-authored notes (2026-07-29) — unacknowledged sends

- An optimistic user turn is now visibly provisional: `ConversationMessage`'s `pending`
  prop ('sending' | 'queued') dims the bubble, dashes its border, and adds a clock marker
  beside it. Acknowledgement is the EXISTING dequeue — `consumedUserCountRef` dropping the
  turn once `session.conversation` carries it — so no new lifecycle was added, only a
  rendering of the one that was already there.
- `pending` is derived positionally in `renderedConversation`:
  `pendingFrom = conversation.length - optimisticMessages.length`, since `conversation` is
  authoritative-then-optimistic and the dequeue is FIFO (`prev.slice(newlyConsumed)`).
- `queued` vs `sending` is captured AT SEND TIME on the turn object
  (`PendingUserTurn = ConversationTurn & { queued?: boolean }`, renderer-local — the daemon
  never sets it) from `!ambientIdleRef.current || pendingCountRef.current > 0`. Note
  `ambientIdle` treats 'background' as idle (the parent turn ended) but NOT
  'waiting_approval' — a turn parked on an approval hasn't finished, so a send during it is
  correctly 'queued'.
- **Do not build the turn object inside a `setOptimisticMessages` updater.** Two failure
  paths (`rawFallback`, the rejected-send path) remove it by object IDENTITY
  (`prev.filter((t) => t !== optimisticTurn)`), so the appended object must be the same one
  the closure captured — which is why the queued flag is computed from `pendingCountRef`
  rather than from the updater's `prev.length`.

## Hand-authored notes (2026-08-13) — skill invocations

- **A `Skill` tool call gets its own card, not a generic tool row.** New
  `components/claude/SkillCard.tsx` (`SKILL_TOOL`, `isSkillCall`, `skillCallName`,
  `SkillCard`) renders the skill name — an openable `FileLink` when the skill is
  file-backed — plus its description, an origin badge and the invocation args. It is
  substituted for `WorkLogEntry` in BOTH work-log styles: `WorkCard.tsx`'s expanded step
  list and `ToolTraceCard.tsx`'s row expansion. `categoryOf` gained a `skill` category so
  the trace chip is not `other`, and `summarizeWork` counts skills ("2 skills") instead of
  filing them under "other".
- **The trap that made this urgent**: the generic fallback in `formatToolSummary`
  (claude-shared.tsx) and in `ToolTraceCard`'s `callTarget` both take "the first string
  value in `tc.input`". A `Skill` call's input is `{ skill, args }`, so any invocation
  carrying `args` rendered `Skill(<the user's argument text>)` where the skill name
  belongs. Both now have an explicit `case 'Skill'`. Any future tool whose input has more
  than one string field needs the same treatment — the fallback is field-order-dependent
  and silently wrong, never empty.
- **Where the description comes from**: a `Skill` tool call carries ONLY the name. The
  session's stream `init` inventory already itemizes every skill (claudemon resolves each
  one's file, origin and frontmatter `description:` — see
  `modules/claude-asset-roots.md`), so `contexts/SkillInventoryContext.tsx`
  (`SkillInventoryProvider` / `useSkillInfo`) exposes it name-keyed and the card looks it
  up rather than inventing a second source. `ClaudePane` wraps its whole render in the
  provider with `session?.statusLine?.capabilities?.inventory?.skills`. Empty on PTY and
  non-Claude sessions (no inventory is reported), where the card degrades to the bare name
  — that path is covered by a test, so don't make the lookup mandatory.
- **The `/` picker's run entries now carry hints.** `slashItems` in `ClaudePane.tsx` built
  ~50 "run" entries straight from `inventory.slashCommands` with the constant hint
  "Run in this session" — fifty unlabelled words, since a name like `batch` or `verify`
  says nothing alone. A `commandHints` map now merges descriptions from two existing
  sources (the library items, which read the same files, and the skill inventory) and falls
  back to the old constant. Note the pre-existing rule in `slashLookup` is unchanged: a
  library skill/command whose title matches a real session command is SKIPPED, because
  invoking the real command beats pasting its text.

## Hand-authored notes (2026-07-29/30) — chat measure, tail-spacer pin, transcript images

- **The centered chat measure is ONE CSS token**: `--wks-chat-width` in `App.css` `:root`
  (900px). Seven surfaces read it (ClaudePane transcript + GUI status row, Composer,
  TasksCard, NeedsYouDock x2, AgentWatchPane) — previously six hardcoded `1040`s plus a
  `900` that had to agree or the composer/dock stop sitting flush under the transcript. It
  is NOT a theme token (`applyTheme()` never touches it); changing chat width is a
  one-line App.css edit. Documented in `apps/desktop/DESIGN_LANGUAGE.md` §4.
- **Send pins your message to the top via a DERIVED tail spacer** (`lib/chatScroll.ts`):
  spacer height = anchorTop − 12 + viewportHeight − contentHeight(excl. spacer), so the
  spacer shrinks by exactly what the streaming reply grows and total scrollHeight holds
  still — the existing sticky-bottom ResizeObserver does the pinning with no scroll math.
  The pin is armed by `handleSend` (`pinArmedRef`), never on mount, so a restored
  transcript opens at its natural bottom. Assertions: `[data-tail-pad]` / `[data-pin-anchor]`
  (jsdom has no layout — only wiring is testable there; geometry was verified in real Chromium).
- **Two distances, two purposes — do not share the answer** (bug found 2026-07-30): the
  scroll-to-bottom BUTTON uses `distanceFromContentEnd()` (discounts the spacer), but
  streaming STICKINESS uses the RAW `scrollHeight - scrollTop - clientHeight`. Sharing the
  discounted one gave the user ~600px of "free" scrolling that still counted as stuck, so
  the next chunk snapped them back with no escape button.
- **Transcript images**: composer thumbnail machinery lives in
  `components/claude/imagePreviews.ts` (module-level path→dataUrl cache + `useImagePreviews`)
  shared by composer chips and transcript tiles (one decode per screenshot).
  `lib/messageImages.ts` is the pure split: `extractImageAttachments()` strips `[Image: /path]`
  markers from USER messages (deliberately LEAVING `[File:]`/`[PDF:]` and non-raster Image
  markers — deleting one erases the only evidence a file was sent); `imagePathsInText()`
  finds absolute image paths in ASSISTANT prose (cap 4) and leaves them in place as
  FileLinks. Tiles render only for paths that actually decoded; `ConversationMessage`
  gained a `cwd` prop purely to resolve relative paths.

## Hand-authored notes (2026-08-23) — a wake's post-bullet BLOCKS need an explicit fold, or the GUI card loses them

`buildFleetMessage` emits post-bullet BLOCKS after a blank line (FAILED note,
STOPPED note, `Structured result — <label> (session:<id>):` + pretty JSON,
`Structured result MISSING — …`, `Full final message — …`), but
`parseFleetMessage`'s bullet loop `break`s at the first non-bullet line — so
**every one of those blocks was dropped**. A dispatch with a `resultSchema`
produced a validated object the manager AGENT could read in the wake text, while
the GUI card showed no trace of it at all: not a raw JSON dump, nothing.
`resultError` was equally invisible.

Fixed 2026-08-23 (`01ad83f7`): `attachResultBlocks()` folds `result`/`resultError`
back onto their entries by `sessionId` and **STOPS at the first
`Full final message — ` block**, because a full reply is arbitrary worker prose
with blank lines whose paragraphs would otherwise be scanned as blocks (and could
FORGE a result). `fullReply` deliberately still does not round-trip.

- **When adding a new extras block to `buildFleetMessage`, add the matching fold
  in `attachResultBlocks` AND a round-trip test in `fleetMessages.test.ts`, or the
  GUI silently loses it.**
- **The `RESULT_MAX` cap means a valid result can arrive as INVALID JSON**
  (truncated mid-object with a `[truncated: …]` marker), so any renderer of
  `entry.result` must tolerate unparseable input.
- Render structured results through
  `apps/desktop/src/renderer/src/components/claude/StructuredResultCard.tsx`
  (+ `structuredResultFields.ts`), which classifies by VALUE SHAPE because the
  schema is authored per dispatch.
