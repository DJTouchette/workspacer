# Intent workflow audit — 2026-09-13

> Historical audit snapshot, before the reactive intent changes prepared for the
> next nightly. Status-only changes now preserve the requirements revision and
> evidence, and activated intents can transition through manager reports and user
> review. The findings and test counts below describe the earlier audited code.
> See the [current guide](intent-workspaces-guide.md) for shipping behavior.

Workspace: `711a920f-9619-44b6-8b7d-db204efbfc3a`, intent revision 2.
Execution: `80687ca1-6661-4f25-80b1-698bf88b499a`.

## Result against the intent

The criterion “The intent feature is clear and precise” is supported by explicit
workflow guidance, a persisted lifecycle regression, and the checks below. This
is engineering evidence for user review, not an automatic acceptance decision.

- **Clear workflows:** the [preview guide](intent-workspaces-guide.md#work-through-pr-review-changes-and-merge)
  now maps defining work, implementation, PR review, addressing comments, merge,
  and reopening to the existing controls. The status editor explains this mapping
  and the consequences of saving a status change.
- **State support:** the stored statuses are Draft, Active, Review, and Complete.
  PR open maps to Review; addressing comments maps to Active; externally confirmed
  merge can lead to Complete when the outcome is finished. No dedicated In PR,
  Addressing comments, Merged, Blocked, or Abandoned states exist.
- **Precise semantics:** status is manual, review acceptance requires verified
  criterion evidence, execution status tracks session association/observations,
  and messaging receipts describe delivery. None establishes another's outcome.

## Findings and changes

1. PR links retain references only. They do not synchronize comments, CI, review,
   or merge events. The guide now states this directly and describes manual rework
   using Direction. Automatic PR transitions would conflict with the current
   convention of explicit personal status and independent external team workflows.
2. Every intent save advances revision, even when only status changes. Accepting
   work and then saving Complete leaves acceptance on the prior revision. Existing
   evidence and review history are retained, but current-revision coverage resets.
   The guide and editor now explain this behavior rather than implying that a
   final status change preserves current acceptance.
3. Complete can be set without evidence and reopened as Active. It neither stops
   agents nor enforces merge or verification. A new integration test exercises PR
   reference retention, Review → Active → Review → Complete → Active, acceptance
   refusal without evidence, historical acceptance, immutable launch context, and
   SQLite restart persistence.

Production behavior remains consistent with the preview's documented manual
status and revision-specific evidence model. The code change adds explanatory
text using existing design tokens and an accessible description for the selector.
The non-obvious revision finding is also recorded in Rivet's learning log.

## Checks actually run

From `apps/desktop` unless indicated otherwise:

| Command | Result |
| --- | --- |
| `npx vitest run src/main/services/intent src/main/shared/intentSummary.test.ts src/main/headless/intentObservations.test.ts` | Initial baseline: 123 passed, 8 Windows-only tests skipped. |
| `npx vitest run src/main/services/intentCompletion.integration.test.ts` | After adding lifecycle regression: 4 passed (3 existing, 1 new). |
| `npx vitest run tests/components/Intent tests/intentExecution.test.ts` from `src/renderer` | 74 passed. |
| `npx vitest run tests/components/IntentWorkspaces.test.tsx` from `src/renderer` | After editor change: 7 passed. |
| `npm run typecheck` | Main and renderer passed. |
| `npm run build:desktop-host` | Passed. |
| `npx playwright test --project=renderer intentCompletion.test.ts --output=/tmp/workspacer-intent-audit-results` | 2 passed: persisted owner-service workflow on desktop/mobile and shell layouts/themes/keyboard navigation. |
| `WKS_DESKTOP_HOST_TEST_BUNDLE=/home/djtouchette/Work/worky/workspacer/apps/desktop/dist/headless/desktop-host.cjs go test ./cmd/brain -run 'TestIntent' -count=1 -v` from `services/hub` | All 4 top-level tests passed, including private host delivery/restart integration. |
| `git diff --check` | Passed. |

Changed TS/TSX files were formatted with Prettier. Rivet context and source were
consulted; `witness.select` returned no test selection, so the relevant existing
suites and production browser workflow were selected directly.

## Limits and unresolved choices

- Windows runtime behavior was not exercised on this Linux host. Browser tests
  use isolated real storage and fixture provider/session activity; this audit did
  not test live tracker credentials, real PR synchronization, or live agent delivery.
- Whether distinct PR stages are needed remains a product choice. The current
  implementation supports those workflows manually, but cannot filter work by
  In PR versus Addressing comments versus Merged as separate stored states.
- Separating lifecycle status changes from intent-content revisions could preserve
  current evidence when only status changes. That would change the persistence
  contract and needs an explicit design; this audit documents and tests today's
  behavior without silently transferring user verification to another revision.
- No checks failed. No live workspace status or user verification was written by
  this audit; its artifacts are available for the user's review.
