# Intent execution completion review

Reviewer entry points: `apps/desktop/src/main/services/intentCompletionStore.ts`
(boundary, immutable proposal and review CAS), `intentWorkspaceStore.ts` (schema v9
and shared request/capture paths), `intentEvidenceStore.ts` (evidence gate and
transaction), and `apps/desktop/src/renderer/src/components/IntentCompletion.tsx`
(human controls). The user workflow is in [the guide](intent-workspaces-guide.md).

The branch preserves reviewed project-integration commits `c0fb7fad` and
`113aa301` as exact ancestors of implementation checkpoint `b83decb8`, based on
`c9d794de`. It adds completion handoff without provider access, a summarizer model,
new delivery infrastructure, or an alternative session lifecycle owner.

## Boundary and persistence

A proposal is approval-eligible only with a current run/execution and requirement
revision, an explicit correlated review report, and owner-host idle without pending
questions/approval, running tools, embedded background tasks or busy descendants.
Stopped/killed, interrupted, stale, missing/malformed/oversized and delivery-unknown
work cannot establish successful completion. Ordinary idle alone is insufficient.
A malformed or absent report is retained for inspection with no completion timestamp.

The completion timestamp is when the owner host observes this boundary, not an
invented provider finish time. Reports retain up to 4,000 UTF-16 units of final
assistant text, preserving formatting except recognizable credential redaction.
Structured fields remain agent assertions; malformed optional fields are omitted
explicitly. No tool input/results or user transcript are added to a proposal.
The headless daemon projection `intent-completion-source/v1` replaces the former
800-character-per-event status summary for this purpose. Older daemons fail closed
with a capture warning. Sparse snapshots cannot restore eligibility after an
interrupted observation.

Proposals and review history are immutable. Review operation IDs are replay-safe;
a changed payload with the same ID is rejected. Current-proposal and revision CAS,
selected user-verified criterion evidence, review insertion and reactive status
transition share the SQLite write transaction. Changes-requested reviews record
the successor direction ID; the existing automation controller delivers it with
accepted/failed/unknown receipts. Unknown delivery is never automatically replayed
or called consumption. Feedback remains recorded if the session cannot resume,
and replacement instructions retain it. New runs/executions/revisions supersede
old proposals without mutating them. Legacy packets remain readable; legacy manual
reviews without a new execution contract do not imply outcome approval.

## Verification (Linux, existing dependencies)

- Full main suite with `--maxWorkers=4`: **4,168 passed, 18 skipped**. Later focused
  completion/automation/workspace/headless regression run: **66 passed**, including
  the added stale/sparse/unknown-delivery cases. Additional focused Git capture,
  completion and headless run: **44 passed**.
- Full renderer suite with Node **22.22.2**, `--maxWorkers=4`: **2,030 passed**.
- Production Playwright `--project=renderer intentCompletion.test.ts`: **3 passed**.
  Covers persistent owner workflows, responsive desktop/mobile widths, themes and
  keyboard tabs, plus synthetic completion → changes requested → one direction →
  new report → explicit user verification → approval → host restart. No live provider.
- Private production desktop-host protocol: **2 passed**, including capture without
  an Execution viewer and persistence after host restart.
- Rust offline completion projection tests: **2 passed**; owner lifecycle tests:
  **49 passed**; existing conversation API tests: **6 passed**. `cargo build --offline`
  and `cargo fmt --check` passed.
- Main and renderer typechecks; main/preload, desktop renderer, web renderer and
  private desktop-host builds passed. Changed TypeScript/TSX/CSS and Playwright
  files pass Prettier; `git diff --check` passes.
- Rivet context and recon dependency/risk inspection preceded edits. CLI
  `witness select` selected 50 main/dependency tests (covered by the full suite).
  The non-obvious report-projection and review-transaction finding is recorded in
  `.rivet/learnings/2026-09-14-intent-completion-needs-a-final-report-projectio-f0c4af.md`.

Node **26.2.0** was used for main/build/protocol/browser checks. Its broad renderer
run failed on unrelated native `localStorage` behavior; Node 22 passed that full
suite. One highly concurrent main run timed out a tombstone-retention test; its
isolated rerun and the four-worker full suite passed. A Git capture fixture had
one transient empty diff during overlapping runs and passed its focused rerun.
Existing Windows-only tests were skipped on Linux. Native Electron GUI and live
provider execution were not exercised; browser production assets and the shared
owner services were exercised with synthetic sessions. No dependencies installed,
physical checkout changes, push, merge, publication or nightly release.
