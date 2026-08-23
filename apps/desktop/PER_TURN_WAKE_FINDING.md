# The per-turn worker-finished wake — reproduced, 2026-08-22

**Verdict: the claim is REAL**, and it is real in two distinct shapes, only one of which is a
defect. A once-per-dispatch latch would be the wrong fix; the evidence says so.

Claim under test (scout `session:955c8a32`, `.workspacer/side-threads-design-2026-08-22.md` §4.1):
`nudgeParentOnFinish` has no once-per-dispatch latch, so any child with a live `isSupervisor` parent
fires a full `[fleet] Worker finished:` wake on **every** working→idle edge.

## Evidence 1 — production transcripts (observational)

Manager `4883a34e-2016-4155-a635-989957ce6dd2`
(`~/.claude/projects/-home-djtouchette/`). Deduped by message `uuid`, so these are distinct
deliveries, not the same wake stored repeatedly.

### 1a. Follow-up-driven repeat — the wake was WANTED

Worker `ca04d576-21fe-4b70-a50a-f396c1159307`:

| time              | event                                                                                              |
| ----------------- | -------------------------------------------------------------------------------------------------- |
| 04:01:03          | `[fleet] Worker finished:` — last reply `⚠️ Error: [ede_diagn…` (an accidental interrupt idled it) |
| 04:01:16          | manager calls `send_message`: _"Disregard the interruption — continue your original task…"_        |
| 04:01:22–04:07:39 | worker works; ~15 approval blocks, manager approves each                                           |
| 04:07:54          | `[fleet] Worker finished:` — last reply `Master hasn't moved…`                                     |

Two working→idle edges, two wakes. **Wake #2 is correct** — the manager sent a follow-up and
needed the answer. Any fix that latches "one wake per dispatch" silently breaks this.

### 1b. Pure duplicate — the wake was NOISE

Worker `7e7f59fd-b9ef-4915-8195-634d0b97a9de`:

| time     | event                                                                                     |
| -------- | ----------------------------------------------------------------------------------------- |
| 16:16:32 | `[fleet] Worker finished:` — last reply `Claude needs your perm…`                         |
| 16:16:38 | manager tries `approve` → HTTP 409 `"session is not awaiting approval", mode: responding` |
| 16:16:43 | `[fleet] Worker finished:` — **same** last reply, 11 s later                              |

No user turn between them, identical payload. The manager's own words in the transcript at
16:16:43: _"Already self-cleared — the fix worker is still running… Not actually finished despite
the wake."_ This one is a straight defect: a block flapping produces a second working→idle edge and
the worker is reported finished twice on a turn it never completed.

## Evidence 2 — in-process reproduction (executable)

`apps/desktop/src/main/services/perTurnWake.repro.test.ts` drives the **real**
`claudeSessionStore` and the **real** `supervisorNudge` with the hook + conversation-delta traffic a
live child produces. Only `claudemonSessionClient.message` — the wire — is mocked, because that is
what is being counted.

- Three conversational turns at a child with a live supervisor parent → **three** wakes at the
  manager, each a parseable `worker-finished` card carrying the `brief.md` tail. The third reports a
  mid-conversation answer (_"A 200ms timeout that CI trips."_) as if it were a dispatch result.
- A `PreToolUse`/`Stop` pair with **no** new prompt → a **second** wake with the identical payload,
  reproducing 1b exactly.

Both pass against unmodified `master`.

## What the code reading got right, and what it missed

Right: there is no latch, and every working→idle edge is a finish
(`claudeSessionStore.ts:508-519`; call sites `:677-678`, `:773-774`, `:985-986`). The only gates are
`hasReceivedTask` (boot idle) and `genuinelyIdle` (a blip that resolved _within_ the 1500 ms coalesce
window). A blip that takes longer than 1500 ms to resume — the 11-second case — sails through.

Missed: the scout framed this as one bug wanting one latch. It is two behaviours sharing a
mechanism, and they pull in opposite directions. Case 1a shows a repeat wake is sometimes the whole
point: _a manager that sends a worker a follow-up instruction must hear back._ A once-per-dispatch
latch would have swallowed `ca04d576`'s real report and left the manager waiting on a worker that
had already answered. The defect is not "more than one wake per child"; it is **a wake for a
working→idle edge nobody is waiting on**.

Also worth recording: the original worry the dispatch raised — wakes going _missing_ — is visible in
the same data (manager `16f18cf2` shows finishes with no wake), and `sweepMissedFinishes` is the
backstop for it. Nothing here weakens that path.
