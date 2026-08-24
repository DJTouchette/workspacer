# Unresolvable approval + lying session state — reproduction findings

Working notes for the two defects in the project brief's `## Now` entry
("BUG — UNRESOLVABLE APPROVAL"). Written down as they were established so the
evidence survives independently of any one session's context.

## 1. The manager's parser hypothesis is WRONG

Hypothesis handed over: *"the out-of-roots denial prints a prompt shape
claudemon's PTY parser does not match"*.

There is **no PTY parser for approvals at all**. Workers spawn on the default
`claude.transport: "stream"` (`configDefaults.generated.ts:182`), i.e. the
headless stream-json managed adapter in
`services/claudemon/src/providers/claude_stream.rs`. Approvals arrive there as
`control_request` / `can_use_tool` frames on stdout — structured JSON, not
rendered text. Nothing greps the PTY.

**Reproduced directly against the real CLI** (`claude 2.1.237`) with a
stand-in driver that speaks the exact argv `build_argv()` produces
(`--print --input-format stream-json --output-format stream-json
--include-partial-messages --verbose --permission-prompt-tool stdio
--session-id … --permission-mode default`), from
`cwd = ~/.workspacer/worktrees/workspacer/<a worktree>`:

| target read | CLI behaviour |
| --- | --- |
| `~/Work/worky/workspacer/.workspacer/brief.md` (the "approvable" control) | `control_request` `can_use_tool`, `decision_reason: "Path is outside allowed working directories"`, `decision_reason_type: "workingDir"` |
| `~/.workspacer/brief.md` (the "hangs" case) | **byte-for-byte the same frame shape**, same `decision_reason`, same `decision_reason_type` |

Both are "outside allowed working directories" — the in-repo control was never
*inside* the worker's cwd either, it was just a different out-of-roots path.
**The CLI does not distinguish them.** So the divergence is downstream of the
CLI, in how workspacer folds that one frame shape into session state, and it is
a race/state bug rather than a prompt-shape bug.

## 2. Where the `can_use_tool` handshake breaks — two independent clobbers

For a stream-transport Claude session the desktop learns about approvals over
**two independent SSE connections** to claudemon, with no ordering guarantee
between them:

* `/events` → `claudemonEventBridge` → `claudeSessionStore.applyManagedMode()`
  → sets `pendingApproval` + `ambientState: 'waiting_approval'`.
  Fed in-process by `claude_stream.rs`'s `surface_approval`.
* `/hooks/stream` → `claudemonHookBridge` → `claudeSessionStore.handleHookEvent()`
  → `sessionStore/hookEventRouter.applyHookEvent()`.
  Fed by the CLI's own `PreToolUse`/`PostToolUse` hooks (a `curl` subprocess →
  claudemon hook port → rebroadcast), so it is the slower, laggier path.

### Clobber A — desktop: hooks null the approval card but not the state

`hookEventRouter.applyHookEvent` is careful to leave `ambientState` alone for
stream sessions (`hooksOwnAmbient = session.transport !== 'stream'`), because
the daemon owns that state machine. But `PreToolUse` (line 114) and
`PostToolUse` (line 153) both do an **unconditional**
`session.pendingApproval = null` regardless of transport.

So a hook frame that lands after the `/events` frame leaves exactly the
reported shape: `ambientState: 'waiting_approval'` with `pendingApproval: null`
— a session the manager can see is blocked and cannot act on, because
`agents.list` / `sessions.snapshot` read `pendingApproval` from this store.

### Clobber B — claudemon: any `Busy` update overwrites a parked approval

`providers/mod.rs::apply_updates`:

```rust
if let Some(mode) = new_mode {
    if mode != *cur_mode || mode == SessionMode::Approval {
        store.set_managed_mode(session_id, mode, pending);  // pending == None for Busy
        *cur_mode = mode;
    }
}
```

`AgentUpdate::Busy` is produced by `translate()` for `system`/`status:requesting`
(claude_stream.rs:113) and `stream_event`/`message_start` (:125). Neither is
guarded against a parked approval, so one arriving while `cur_mode ==
Approval` sets mode `Responding` **and** `pending: None` — the session then
reports `streaming` while the CLI is still blocked on an unanswered
`can_use_tool`, and the approval card is gone. That is defect 2 *and* a second
route into defect 1.

The two `background_tasks_changed` paths right below in `claude_stream.rs`
(:1616, :1637) are explicitly guarded for exactly this reason ("Guarded to
Input so a parked approval/question is never clobbered"). `apply_updates` is
the same hazard with no guard, and it is shared by *every* managed provider
(codex/opencode/pi), not just claude.

## 3. Third, independent defect: PTY sessions get no approval record at all

`session/state.rs:53` claims `PermissionRequest` is "NOT a real registerable
hook — internal / forward-compat only", so `HookEventKind::REGISTERABLE` omits
it and `daemon/init.rs::HOOK_EVENTS` never installs it. But `PermissionRequest`
**is** a real Claude Code hook event today (it is present in the live
`~/.claude/settings.json` wired to another tool), and it is the *only* thing
that can set `SessionMode::Approval` for a PTY-transport claude session
(`state.rs:684`). A PTY worker therefore never produces an approvable record —
the same unresolvable hang by a different route.

## Repro harness

`/tmp/wks-repro/driver.py <cwd> <prompt> [--answer allow|deny|none]` — spawns the
real CLI with claudemon's argv under an isolated `CLAUDE_CONFIG_DIR`
(`/tmp/wks-repro/cfg`, credentials symlinked, no hooks) so nothing lands in the
live daemon, and logs every frame with timestamps.

## Resolution

All three defects are fixed on this branch. Kept as a document rather than
deleted with the last commit, because the two WRONG hypotheses above (a PTY
prompt-shape parser; something special about the `~/.workspacer/` path) are the
ones a future reader will re-derive first, and section 1 is the evidence that
they are dead ends.

| Defect | Fix | Test |
| --- | --- | --- |
| A — hooks null the card they don't own | `hookEventRouter.ts`: `clearPendingApproval()` gates the PreToolUse/PostToolUse clears on `hooksOwnPending`, the mirror of the store's `daemonOwnsPending` | `hookEventRouter.test.ts` — "a late hook frame must not null an approval the daemon still holds" |
| B — `Busy` demotes a parked approval | `providers/mod.rs::apply_updates`: `Busy` no longer raises `Responding` out of `Approval`/`Question` | `busy_never_demotes_a_parked_approval_or_question`, `approval_still_lands_and_a_real_turn_end_still_clears_it` |
| C — `PermissionRequest` never registered | `session/state.rs`: added to `HookEventKind::REGISTERABLE`, so `daemon/init.rs` installs it | `contracts/permission-request-hook-cases.json`, read by `permission_request_contract_cases` (Rust) and `permissionRequestContract.test.ts` (TS) |

The invariant the three share, and the thing to preserve: **a session's
`pending` slot may only be cleared by the feed that raised what is on it;
another feed may enrich or displace it, but never destroy it.** For a
stream-transport claude session that owner is the daemon's `/events`
managed-mode stream; for a PTY claude session it is the hook feed. Mixing them
is what made a blocked session unanswerable rather than merely mislabelled.

*(The stronger form — "exactly one feed owns the slot" — was how this was
written until 2026-08-23. It is false for codex, opencode and pi, which carry a
second writer by design; see "The second writer" below.)*

### The class, swept (2026-08-23)

Two more commits landed against this invariant after the table above:
`aae765a3` (a `send_message` must not wipe a parked approval, in
`claude_stream::note_user_send`) and then a full sweep of `services/claudemon`.

The sweep's finding is that the invariant was being enforced by a hand-written
condition repeated at every call site —

```rust
if cur_mode != SessionMode::Approval && cur_mode != SessionMode::Question {
    store.set_managed_mode(id, SessionMode::Responding, None);
}
```

— and that **three more drivers had the same send arm with the guard missing**,
each reachable exactly like the claude one:

| Site | Shape |
| --- | --- |
| `opencode.rs`, `rx.recv()` arm | `if cur_mode != Responding { set_managed_mode(Responding, None) }` — true while a permission is parked |
| `codex.rs`, `rx.recv()` arm | same |
| `pi.rs`, `rx.recv()` arm | same |

Plus two pi-only defects of the same family: it surfaced the **newest** parked
dialog while the decision arm answered the **FIFO front** (user approves one
card, a different request gets their answer), and answering a dialog never
re-surfaced the next one — so with two dialogs in flight, answering one cleared
the card while pi stayed blocked on the other. That is the unresolvable-approval
shape reached through pi.

**The fix is structural rather than a fourth, fifth and sixth `if`.**
`SessionStore::set_managed_mode` no longer takes a bare `Option<Pending>`; it
takes a [`PendingWrite`] saying what the write MEANS:

* `Park(owner, pending)` — `owner` raising a block. Always applies.
* `Resolve(owner)` — `owner`'s request is over (the user answered, or a genuine
  turn boundary arrived). Clears only what `owner` raised.
* `Keep` — liveness/enrichment (a busy ping, a message written to stdin, a
  background-task count). Suppressed **entirely** while a request is parked,
  mode included; it carries no owner because it may never touch the slot
  whoever sends it.

The guard now lives in one place and cannot be forgotten: a new call site must
say which of the three it is and on whose behalf, and the compiler asks. (The
`owner` argument arrived later, on 2026-08-23 — see below. The first version of
this type fenced only `Keep`.) `set_managed_mode` returns
what the store actually holds (not what was asked for), and drivers mirror
their `cur_mode` from it via `providers::set_mode`, so a suppressed write can't
leave a driver believing a mode the store never adopted. The four drivers' send
arms are now one shared `providers::note_user_send`.

`ingest`'s existing rule — hooks are enrichment-only for a managed/stream
session, `session/store.rs` — turns out to be why items 1 and 2 below do NOT
hold inside claudemon. It had no test; it has three now.

### The second writer: `mcp_ask` (2026-08-23)

The `Keep`-only fence above closed the class **for claude**. For **codex,
opencode and pi it did not**, because those three have a second feed writing
the same slot: `daemon/mcp_ask.rs`, the `AskUserQuestion` MCP endpoint spawned
for them (`codex.rs` `--config mcp_servers`, `opencode.rs` `ask_mcp_entry`,
`pi.rs` `ask_extension_source`) because they have no native structured-question
tool. It parked and resolved unconditionally, since `PendingWrite` only fenced
`Keep`.

Reachable shape, verified by test before the fix (both assertions read
`pending is None`):

1. the driver parks an approval card and holds the request id in its own FIFO;
2. the agent calls `AskUserQuestion` in the same turn — the question's `Park`
   overwrites the approval card;
3. `QuestionGuard::finish` (or its `Drop`, on a killed agent) `Resolve`s and
   clears the slot.

The driver's FIFO still holds the approval, and it only re-surfaces a queued
card from its `/approve` decision arm — a decision that can only arrive for a
card the user can still see. Session wedged: the exact shape of this document,
reached across two feeds rather than within one. Narrow (needs a concurrent
approval and question) and pre-existing, not caused by the refactor — but the
refactor's own doc comments asserted the class was closed.

**Fix: the write intent names its feed.** `PendingWrite::Park` and
`::Resolve` now carry a `PendingOwner` (`Primary` = the hook feed or the
session's driver task; `Ask` = the MCP shim), so the store can tell "my request
is over" from "someone else's request is on the card":

* a park by the other feed **displaces** the card instead of overwriting it,
  and the displaced request is restored — under the mode it was parked with —
  when the displacing one is released;
* a resolve clears only what its own feed raised; finding the other feed's card
  it changes nothing visible (and drops only its own displaced request, whose
  block is genuinely over), so the mode keeps reporting the block that is
  actually live.

Structural again, not a fourth condition: the owner is a payload the compiler
demands at every call site, old and new. The slot itself is now **private**
(`SessionState::pending_card`, read via `pending()`), so the only way to write
it at all is `write_pending`, which is where every rule lives. That privacy
turned up one more writer the earlier censuses had missed —
`SessionStore::park_decision`, the PTY hook gateway — which now parks through
the funnel as `Primary`. It had no rival feed (no `mcp_ask` is registered for a
PTY session), so it was not a live defect; going through the funnel is what
keeps that true rather than merely observed.

Tests: `an_answered_ask_leaves_a_driver_approval_it_did_not_park_intact` and
`an_aborted_ask_leaves_a_driver_approval_it_did_not_park_intact` (mcp_ask.rs),
`a_park_from_the_other_feed_displaces_the_card_and_its_resolve_restores_it`,
`a_resolve_from_a_feed_that_owns_nothing_leaves_the_card_and_mode_alone`,
`a_foreign_resolve_still_drops_the_releasing_feeds_displaced_request`,
`a_same_feed_park_replaces_without_displacing` (store.rs). The drop-guard test
also settles an earlier open question — "what if a non-question request is
parked when `QuestionGuard` drops" — as **reachable today, and now harmless**.

Still open, same feed rather than across feeds: two *concurrent* asks on one
session share one answer channel (`register_managed_answer` is
last-writer-wins), so the earlier ask waits out its six-hour timeout. No
supported agent emits parallel `AskUserQuestion` calls today, and closing it
needs a per-ask channel, not an ownership rule.

### Known remaining asymmetries

Both are in **`apps/desktop/src/main/services/sessionStore/hookEventRouter.ts`**
and were re-verified as live on 2026-08-23. Neither has a claudemon analogue:
the daemon's `ingest` returns early for managed/stream sessions before
`SessionState::apply` runs, so no hook can reach a driver-owned pending slot
(`hooks_never_touch_the_pending_slot_of_a_daemon_owned_session`).

* `pendingQuestions` is still written (`PreToolUse`, line ~160) and cleared
  (`PostToolUse`, ~205) from the hook feed on every transport. Narrower than
  the approval case — the clear requires a matching `AskUserQuestion`
  `tool_use_id` — but it is the same shape of shared ownership, and unlike
  `pendingApproval` it has no `hooksOwnPending` gate at all.
* `PermissionRequest` (~222) still writes `pendingApproval` on a daemon-owned
  session. It can only ever ADD a card, so it cannot strand a session, but a
  late frame can resurrect one the daemon already cleared, and the daemon
  deliberately does not surface queued (non-head) approvals that this hook
  would.

* `applyStopEvent` still clears the card on both transports. Deliberate: a turn
  boundary really does mean nothing can still be parked, and a stream session
  killed mid-approval would otherwise keep a phantom card (the daemon's
  `stopped` mode leaves state as-is).
