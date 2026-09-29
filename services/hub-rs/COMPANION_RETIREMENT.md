# Private companion removal plan

Reviewed against the current reference sources on 2026-09-29. This is a deletion
plan, not a completed cutover gate. No reference source has been deleted.

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

## Remove protocol/build paths in this order

1. Replace the two explicit package scripts `build:desktop-host` and
   `test:desktop-host`; neither is needed by the current default release build.
   The latter still invokes `scripts/test-desktop-services.mjs`, which runs Go
   with `WKS_DESKTOP_HOST_TEST_BUNDLE`. These are real remaining build paths,
   not documentation references. Give the replacement contract suite an honest
   name such as `test:desktop-services`, using Rust tests plus Electron tests.
2. Once assertions below are covered, retire `scripts/build-desktop-host.mjs`,
   `scripts/test-desktop-services.mjs`, `scripts/test-desktop-host.mjs`, and
   `scripts/test-headless-analytics.mjs`. The build script's Electron-import
   prohibition and stdout banner only protect the retired private bundle.
3. Remove `headless/stdio.ts`: stdin JSONL parsing, callback/result framing,
   stdout ownership, EOF waiting and its private cleanup scheduler are wholly
   companion transport. Rust owned runtime shutdown replaces that transport;
   do not recreate a Node-compatible subprocess just to preserve its framing.
4. Strip companion-only dispatch from the shared `desktopHost.ts` before
   deleting its imports. The private `internal.*` cases cover prepare/accept/
   cancel spawn, launch integrations, replacement routing, analytics snapshots,
   workflow request, delivery receipts, result commits and observations. Their
   only production caller is the Go/private stdio path. Preserve public cases
   and helper state still referenced by those public cases; this needs an
   import/use check, not a line-range deletion.
5. The public `desktop.managerReplacement` fallback in that shared dispatcher
   also references `headless/managerReplacement.ts`. Electron's native wrapper
   already intercepts this exact method with its native service. Remove the
   redundant fallback during extraction and keep the manifest test looking at
   both wrapper and dispatcher. Then remove `headless/managerReplacement.ts`
   and `headless/hostBridge.ts`; the latter writes `hostCallId` to stdout and
   has no valid Electron transport. Remove `headless/analytics.ts` after its
   `internal.analytics*` callers disappear; retain Electron's separate analytics
   services and shared pricing/parser code.
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
| Add account and enumerate its auth configuration | `tests/profiles.rs`, `src/services/account_setup.rs` tests | Review exact add/list assertion when removing the monolithic reference test; registration alone is insufficient |
| Manager request prepare, duplicate begin suppression, confirmed receipt | `tests/local_spawn.rs`, `src/services/manager_requests.rs` tests | Preserve explicit receipt/unknown-outcome tests |
| Allocate worktree, accept tracked worker, capture committed diff, reject forged owner, evidence survives worktree removal | `tests/fleet_review.rs` actual Git/engine/wakes path; `tests/local_spawn.rs`; `tests/worker_results.rs` | Existing portable coverage is more decomposed than the old single Node test; keep all relevant targets |
| Brief board has lanes | `tests/briefs.rs` reference-card and board mutation fixtures | Preserve tests |
| Retired intent-workspaces SQLite file/artifact unchanged, no WAL created, old request refused despite config opt-in | `tests/legacy_preservation.rs` now implements two isolated actual Backend starts against a real legacy SQLite fixture | Passed the isolated actual-backend fixture on Linux. Checks database/artifact bytes and absent WAL/SHM/journal around startup, current public service calls, retired-method rejection and shutdown |
| Analytics model split, managed provider cost, filters, persistence after transcript removal and empty daemon inventory | `tests/analytics.rs::persistent_headless_analytics_retains_deduplicated_model_splits_and_filters` uses the SAME `contracts/analytics-history-cases.json` | Old explicit `gpt-5.costUSD == 0.75` assertion has now been ported and passed in the analytics five-test target. Reopening SQLite already verifies persistence; process JSONL framing itself retires |
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
