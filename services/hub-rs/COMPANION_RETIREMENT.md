# Private companion removal plan

Reviewed against the reference sources on 2026-09-29. Steps 1–5 below are now
implemented: the private Node entry point, callback adapters and bundle scripts
are removed; the Electron public dispatcher remains. Go source deletion and
final platform cutover remain root-owned gates.

## Keep Electron's shared implementations

`apps/desktop/src/main/services/nativeDesktopServices.ts` directly imports
`desktopHostCall`, `configureNativeDesktopRuntime`, and `HostContext` from
`headless/desktopHost.ts`. It installs Electron's existing workflow controllers
and session lookup, intercepts manager replacement/readiness/request delivery,
and delegates the remaining public service methods to that dispatcher.

Keep `headless/desktopHost.ts`, `headless/files.ts`, and `headless/uiAssets.ts`
until the public dispatcher is deliberately extracted or renamed. Keep their
shared `services/*`, `shared/*`, and `lib/*` dependencies. In particular,
`services/managerReplacement*`, workflow services, analytics/pricing, account
setup and provider completion services are Electron functionality, not disposable
companion code. The directory name alone is not a deletion criterion.

`src/main/shared/desktopServices.test.ts` intentionally verifies that every
public manifest method exists in Electron's fixed dispatcher and Rust's catalog.
Keep that test and `scripts/gen-desktop-services.mjs`; the generator now produces
only the shared TypeScript registration and does not require Go.

## Implemented transport/build removal

1. Removed `build:desktop-host` and `test:desktop-host` package scripts. The
   replacement `test:desktop-services` runs named Rust behavioral targets and
   retained Electron tests. Its rewritten runner validates every test path exists;
   it never builds a private bundle or invokes Go.
2. Removed `scripts/build-desktop-host.mjs`, `scripts/test-desktop-host.mjs`, and
   `scripts/test-headless-analytics.mjs`. Their business assertions map to the
   portable evidence below. The old Electron-import prohibition and stdout
   banner protected only the retired bundle. A tested `prebuild:main` step now
   clears emitted main JavaScript and exact old CJS/map outputs, preventing
   stale incremental packages from retaining deleted transport.
3. Removed `headless/stdio.ts`: stdin JSONL parsing, callback/result framing,
   stdout ownership, EOF waiting and its private cleanup scheduler. Rust owned
   runtime shutdown replaces that transport without a compatibility subprocess.
4. Removed all companion-only `internal.*` switch clauses and their unused
   imports/state from the shared `desktopHost.ts`. Public cases and their used
   helpers remain. This includes retained Electron workflow conversation reads,
   file/UI assets, profiles, pricing, briefs and task/review services.
5. Removed the duplicate `desktop.managerReplacement` fallback: Electron's native
   wrapper already intercepts this method with its native service. Removed
   `headless/managerReplacement.ts`, `headless/hostBridge.ts` and
   `headless/analytics.ts` after their private callers disappeared. Electron's
   separate replacement/analytics controllers and shared parsers remain.

## Remaining root-owned removal

6. Root removes `services/hub/cmd/brain/desktophost.go` and its companion-dependent
   integration tests with the final Go retirement. Eliminate its lazy `node`
   startup, `WKS_DESKTOP_HOST` lookup and child-generation callback protocol.
   Current Rust runtime has no corresponding Node/CJS fallback.
7. Remove obsolete ignored CJS artifact paths and build output. Do not remove
   negative packaging fixtures: `apps/native/scripts/windows-payload.test.mjs`
   deliberately creates old Go/CJS/Node files and asserts they are NOT packaged.
   External provider/plugin Node runtimes remain allowed and required where used.

## Assertion-level portable evidence

| Reference assertion | Existing Rust evidence | Remaining action before retiring that assertion |
| --- | --- | --- |
| Font install/list/read, invalid header/traversal/symlink refusal, arbitrary owner file read | `tests/ui_assets.rs`, `tests/files.rs`; actual view/owner bus boundary in `display_assets_are_viewable_but_installation_remains_owner_only` | Preserve tests; no CJS dependency |
| Pricing defaults, valid override persistence, invalid negative rates | `tests/analytics.rs::pricing_roundtrip_and_shared_cost_contract` | Preserve tests |
| Add account and enumerate its auth configuration | `tests/legacy_preservation.rs::account_creation_is_visible_through_the_public_account_list`, plus profile/account setup tests | Real owned backend public add/list roundtrip passed; subprocess environment confines account root to a temporary home |
| Manager request prepare, duplicate begin suppression, confirmed receipt | `tests/local_spawn.rs`, `src/services/manager_requests.rs` tests | Preserve explicit receipt/unknown-outcome tests |
| Allocate worktree, accept tracked worker, capture committed diff, reject forged owner, evidence survives worktree removal | `tests/fleet_review.rs` actual Git/engine/wakes path; `tests/local_spawn.rs`; `tests/worker_results.rs` | Existing portable coverage is more decomposed than the old single Node test; keep all relevant targets |
| Brief board has lanes | `tests/briefs.rs` reference-card and board mutation fixtures | Preserve tests |
| Retired intent-workspaces SQLite file/artifact unchanged, no WAL created, old request refused despite config opt-in | `tests/legacy_preservation.rs` now implements two isolated actual Backend starts against a real legacy SQLite fixture | Passed the isolated actual-backend fixture on Linux. Checks database/artifact bytes and absent WAL/SHM/journal around startup, current public service calls, retired-method rejection and shutdown |
| Analytics model split, managed provider cost, filters, persistence after transcript removal and empty daemon inventory | `tests/analytics.rs::persistent_headless_analytics_retains_deduplicated_model_splits_and_filters` and retained Electron `analyticsHistoryContract.test.ts` use the SAME `contracts/analytics-history-cases.json` | Rust analytics5 and actual Electron SQLite corpus1 passed, including managed model cost0.75. The Electron test uses its production store/parser/schema; Rust additionally tests snapshot-to-record conversion. Reopening SQLite verifies persistence; process JSONL framing itself retires |
| Go→Node integration renders template with actual execution cwd, propagates schema, validates result, rejects forged task before daemon spawn | `tests/local_spawn.rs`, `tests/workflows.rs`, `tests/worker_results.rs`, `tests/fleet_review.rs` | Verify these combined targets, not just public method inventory |
| Private plugin callback retains spawn identity and patches final launch args/env | `src/plugins/launch/tests.rs` bound-permit, context-only callback, revoked reply and delayed cancellation tests | Preserve; private actor proof replaces stdio callback IDs |
| Manager replacement transfers tasks/worker metadata, holds messages until viewer bind, retires predecessor and delivers kickoff+held message | `tests/manager_replacements.rs::complete_handoff_commits_tasks_before_binding_and_requires_actual_view_ack`, `tests/local_spawn.rs` | Preserve real admission/receipt regression and deterministic state tests; no need to recreate Go child |

## Final verification

Run Rust portable service targets above, desktop typecheck and shared manifest /
backend parity / manager replacement tests after extraction. Build Electron to
prove the retained public dispatcher has no missing imports. Run packaging
negative tests and default source/artifact container boot contracts. Search
executable/build source for `desktop-host.cjs`, `WKS_DESKTOP_HOST`, and the old
build scripts; distinguish negative fixtures from live fallback. Gate final
removal on actual platform CI rather than treating a Linux typecheck as Windows
process-ownership evidence.

Validation at this change: Rust legacy/account2, analytics5; retained Electron
service96, downstream capability/spawn395, source retirement guard2, output
cleanup1 and retained analytics corpus1 passed. Full main/renderer typecheck
and main production build passed. Full renderer/web production build also passed; final platform CI remains
a separate gate.

The final Go companion-boundary review additionally exposed and closed two Rust
behavior gaps: rendered receipts now truncate at 16,000 Unicode scalar values
(as Go `[]rune` does), and an acknowledged launch whose first prompt was not
queued gets one owned fallback delivery. Unknown spawn acknowledgement never
triggers that fallback; message uncertainty preserves the accepted session receipt.
`agent_spawn` passed all 14 tests, including forged-owner rejection before any
engine/worktree effect. `manager_replacements` passed all 16 tests, including
production kickoff text, exact held-message content plus its parent correction,
committed worker/task ownership, predecessor retirement and no duplicate delivery
on repeated binding. Final migration-ledger updates remain separately reviewed.
