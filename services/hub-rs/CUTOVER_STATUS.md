# Rust backend cutover evidence

## Current checkpoint — 2026-09-30

The source ledger contains **505 ported, 29 retired and zero pending** entries.
**All 14 cutover gates are verified.** The tracked Go backend has been removed;
original source hashes, captured fixtures and scoped evidence remain.

[Non-deletion receipts](reviews/cutover-verification.json) cover the consumer,
platform and package contracts. [Deletion receipts](reviews/legacy-deletion-verification.json)
record successful post-removal primary CI, all three native clients and container
validation at `96e2568a`. The scanner and corpus guards preserve sealed historical
identities while continuing to require every active Rust/TypeScript loader.

`python3 scripts/hub-migration.py ready` passes. The shared build milestone now
reports `migrationComplete:true`; live readiness and provider availability remain
separate. Completion-indicator changes still go through normal CI before release.
A published artifact's source revision is identified by its release tag and notes,
not inferred from the current source tree.

## Verified artifact and integration checkpoints

The earlier verified published nightly checkpoint is **0.169.0-nightly.202609300525**, from
`de2687fc2db35d1d3ea25fff3f89d18bfb4806a8`.
[Release36673333608](https://github.com/DJTouchette/workspacer/actions/runs/36673333608)
passed all three package legs, modern/legacy MCP catalogs, real packaged Electron
owned/adopted startup and shutdown, and native Windows installation/upgrade/
backend/uninstall. The tag and all three updater YAML asset names/sizes were
checked after publication. This artifact contains the latest native UI commit
`7348414f`; the later web publication/portable-cutover batch is not in it.

That later batch passed full [CI36673551314](https://github.com/DJTouchette/workspacer/actions/runs/36673551314)
and [container36674585697](https://github.com/DJTouchette/workspacer/actions/runs/36674585697)
at `01888095`. Main integrated it as `5809551b`; only three contributor documents
changed between those revisions. Actual TS-writer task/review/replacement imports,
reopen/recovery and broker publication/refusal now have executing Rust/browser
evidence. Native tests, TUI reconnect/owned shutdown and the published package
receipts retain their exact source revisions and claim limits.

Original source hashes and all 534 review records remain. The optional historical
oracle requires the sealed pinned checkout; it does not fall back to current
Go files. [Non-Go disposition](reviews/legacy-asset-disposition.json) records
32 retained active files and six historical-only inputs. Tracked removal leaves
any ignored local build artifacts untouched; these are not shipped inputs.

The earlier audits below are historical context, not the current backlog.
Run `python3 scripts/hub-migration.py backlog` for the live source/gate count.

## Historical audit — 2026-09-29

The following checkpoint predates the published nightly above and subsequent
main commits. Its implementation/evidence boundaries remain useful; its counts,
publication state and statements that fixes require a new build are historical.

This is an audit of checkout `1bf2f53aeff4f08e5a65f0a58d5b42908269d908`
and its uncommitted integration work. It does not mark any migration gate verified.
The source review inventory is separate from runtime and packaging evidence.
At inspection, `python3 scripts/hub-migration.py check` succeeded with 534 rows:
300 ported, 26 retired, 208 pending; all 14 cutover gates were pending.
Pending rows do not imply that their Rust implementations are missing.

## Integration checkpoint after the audit

The manager recorded the jobs/quiescence review batches, upload-store review and
brain-parameter scanner replacement. The current ledger has **312 ported,
26 retired and 196 pending** entries; all 14 cutover gates remain pending.
The scanner now verifies all 84 captured Go caller bindings individually and
reports 122 methods, 126 dangerous bindings, 13 opaque methods and zero errors.
`make check-hub-capability-parameters` runs its formatting/tests/source check in CI.

Windows testing of the published candidate found a protocol-specific MCP failure:
modern `tools/list` omitted required cache metadata although `/health` was good.
The Rust adapter now supplies private, zero-TTL metadata for every list family;
real HTTP tests cover modern and legacy catalogs, scopes and calls. The packaged
server smoke now checks actual modern and legacy tool lists in addition to service
health. The published archive's earlier health-only success is not evidence that
its tool list works with MCP 2026-07-28. These fixes require a new build.

The Opus 5.5 resolver now uses Anthropic's documented default 1M window instead
of falling through to the generic Claude 200K row. Shared Rust/TypeScript/Go
contracts retain runtime-reported values, user overrides and drift refusal.

Local validation checkpoints: hub library312 + jobs8 + quiescence3;
MCP13 + legacy SSE4; hub models4 + config20 + spawn-plan8; scanner15 +
TypeScript parameter guards19; model TypeScript170 and Rust exact-source
window-module9; sweep accounting19; packaged smoke regression6. These are
separate targeted runs, not a claim of one complete platform suite.

## Published candidate and evidence boundaries

The handoff's in-progress publication has finished:

- [Release run 36596973339](https://github.com/DJTouchette/workspacer/actions/runs/36596973339)
  completed successfully. Windows native packaging and installation/backend/uninstall
  smoke succeeded; publication finished at 17:03 UTC.
- The [nightly release](https://github.com/DJTouchette/workspacer/releases/tag/nightly)
  was published at `2026-09-29T17:02:58Z`. The tag resolves directly to the exact
  candidate SHA above. It contains Electron Windows/macOS/Linux artifacts, the
  separate Native Rust Preview Windows installer, three server bundles, three
  claudemon bundles, and `latest.yml`, `latest-linux.yml`, `latest-mac.yml`.
- [CI run 36595813246](https://github.com/DJTouchette/workspacer/actions/runs/36595813246)
  succeeded on that same SHA. This covers Linux backend/TUI/desktop checks,
  browser/mobile and dispatch integration, and Windows backend/process contracts.
- The exact-SHA run listing contained only those CI and release runs. Native-client,
  Rust container contract and Rust native preview therefore have no exact-candidate
  run receipt. Their path filters do not run on every release-policy-only change.
  Earlier results are useful checkpoints, but require an explicit source-scope review
  or final integrated revision rerun. Dirty jobs/quiescence/scanner work is absent
  from the published artifacts.

These are build/test/publication receipts, not claims of full application parity.
Native macOS/Linux installers are not supplied by this release workflow. Native
Windows installation smoke uses the backend harness; it is not a visual launch
of the installed GPUI executable.

## Gate-by-gate work map

Paths below are evidence to review, not automatic certification. Test files named
here were inspected as available suites; this audit did not execute heavy builds.

| Pending gate | Existing implementation / evidence | What remains before recording verified |
| --- | --- | --- |
| `electron-and-web` | `apps/desktop/src/main/services/hubDaemon.ts` selects Rust with no Go fallback; `build-rust-backend.mjs` and `electron-builder.yml` package it. `ci.yml` runs desktop tests, dispatch-chain and browser/mobile e2e against Rust fixtures. | Review remaining client contract rows; record successful final-revision client integration and packaged Electron startup/shutdown receipts. Browser fixture success alone does not certify installed Electron ownership. |
| `native-embedded` | Native Cargo defaults include `rust-hub`; shared `backend.rs` and native adapter own lifecycle. `native-client.yml` tests three OSes and real Linux window smoke; release Windows backend/installer smoke succeeded. | Final-revision native matrix and owned embedding receipts; document the distinction between fixture window, backend harness and installed GUI checks. Resolve remaining native contract review gaps. |
| `standalone-service` | `src/cli/serve.rs`, `backend.rs`, `tests/cli.rs`, `tests/backend_owner.rs`, `tests/shutdown.rs`, `CLI_MIGRATION.md`; release produced all three standalone server bundles. | Record final packaged CLI startup/readiness/shutdown, token provenance and state-path checks on supported platforms. Review outstanding standalone behavior rows. |
| `tui` | `apps/tui/src/daemons.rs` bootstraps Rust for a missing local bus, preserves external ownership and supports claudemon-direct fallback. `ci.yml` runs TUI format/tests/clippy. | Record real Rust-bus TUI connection, session/event operations, reconnect and owned shutdown evidence. Unit tests of bootstrap selection are not a complete TUI-to-backend smoke. |
| `mcp-and-plugins` | `tests/mcp.rs`, `mcp_sse.rs`, `plugins.rs`, `plugin_manager.rs`, `auth_composition.rs` and desktop dispatch-chain tests exist. | Finish per-method parameter/caller audit and original Go binding capture; review dynamic plugin catalogs, identity propagation, cancellation and lifecycle against retained contracts, then record final integration evidence. |
| `federation-and-remote-workers` | `src/federation.rs`, `tests/federation.rs`, `remote_client.rs`, `worker_results.rs`, `fleet_messages.rs` and deployment worker boot contracts exist. | Review remaining source rows; record final hub/worker authorization, reconnection, remote dispatch/result and restart coverage. Container readiness alone is not an end-to-end remote workflow. |
| `persisted-state-upgrade` | `tests/legacy_preservation.rs`, `stores.rs`, `config.rs`, `layout.rs`, jobs/history tests and CLI identity/path tests cover individual upgrade contracts. Default standalone hub state retains the legacy directory. Native preview uses isolated state. | Establish a reviewed inventory of persisted families, match each to fixture/restart assertions, and retain explicit identity-loss/error-path tests. A successful fresh-volume boot does not establish upgrade safety. Integrate the jobs history fix before final evidence. |
| `node-companion-replacement` | Electron payload selects Rust plus standalone claudemon; native payload contains `wks-native.exe`, `workspacer-rust.exe` and CRT files. Windows harness smoke removes Node from PATH. | Record package/runtime evidence that private companion duties are all supplied by Rust. Keep optional external provider/plugin runtimes distinct from backend dependencies. Update stale Go/private-companion operational docs. |
| `windows-package` | Exact-candidate Electron package, native installer, native install/upgrade/backend/uninstall smoke and standalone bundles succeeded in release run 36596973339. | Re-run for the final integrated revision, review payload/identity/update metadata and record receipt. Any later cache/job split requires its own nonpublishing validation. |
| `macos-package` | Exact-candidate arm64 Electron DMG and standalone bundles built and uploaded successfully. Automatic Electron updates are intentionally unsupported on macOS. | Final-revision packaging and applicable launch/service smoke evidence. Do not interpret DMG construction as installed-app behavior or invent a native macOS installer requirement. |
| `linux-package` | Exact-candidate x64 AppImage and standalone bundles built and uploaded successfully. Linux CI runs backend/browser tests. | Final-revision packaged startup/shutdown and package checks, with exact build receipt recorded. No native Linux installer is currently part of release packaging. |
| `deployment` | Default Fly Dockerfiles build Rust. `rust-container-contract.yml` checks generation, all role images, persistence/artifact/supervisor contracts, explicit credential migration and real fresh-volume boot/shutdown. Ledger checkpoint `379d3b34` explicitly requires a final-head rerun. | Final integrated SHA image/boot/upgrade-contract receipt. Document simulated Tailscale versus real backend execution; do not claim cloud rollout or existing-volume upgrade solely from fresh-volume boot. |
| `ci-and-generators` | `ci.yml` uses Rust backend tests and Rust browser fixtures; config defaults generator uses `services/hub-rs/assets/config-defaults.json`. Portable parity/reference commands remain in Makefile. | Wire and pass the completed capability source scanner; finalize portable generated inputs before removal of Go reference generators; run final platform workflows and record exact-SHA evidence. |
| `legacy-deletion` | Original implementation retained under `services/hub`; reviewed replacement records and portable fixture files already exist. | Finish every source review and other cutover gate first. Preserve/move non-Go assets and examples, retire reference-only commands, remove legacy backend source, verify no build/runtime dependency remains, then record deletion evidence and pass `ready`. |

## Concrete work that is still missing

1. Validate the integrated jobs/quiescence, parameter scanner and reported-runtime
   fixes against the final candidate in CI and packaged smoke tests. The handoff's
   39 scanner issues have been resolved; the original bindings and source hashes
   now have a dedicated CI guard.
2. Review the remaining source inventory by behavior and actual regression guards.
   Aggregate passing tests cannot certify an unreviewed source file.
3. Assemble final-revision gate receipts. CI, native-client, native preview,
   container contracts and release packaging have different scopes; successful
   release publication does not transitively require all those independent workflows.
4. Prepare deletion without breaking retained assets. In particular,
   `apps/desktop/electron-builder.yml`, `apps/native/scripts/windows-payload.mjs`
   and `release.yml` copy `services/hub/examples`. Desktop `test:headroom` also
   reads that directory. Deleting the whole legacy directory would break valid
   plugin/example packaging even though the runtime binaries are Rust.
5. Refresh operational docs. This audit corrected `MIGRATION.md` statements that
   no nightly was published, production used the old backend and native Rust was
   opt-in. `workspacer-serve-cli` context still describes the Go child stack/private
   Node companion; subsystem ownership docs need their own focused refresh.

## Completion-check limitations

`scripts/hub-migration.py ready` checks recorded legacy hashes, existing replacement
and test paths, every gate's `verified` status and nonempty gate evidence. It does
not execute tests, validate CI SHA/conclusions or prove legacy files were deleted.
There is no specialized gate-recording subcommand. The root reviewer must preserve
concrete, reviewed receipts rather than treating a nonempty evidence array as proof.

The manifest includes `legacy-deletion` among its required verified gates while
the script's error message says `ready` must pass before Go removal. Interpret
this conservatively: complete/review all non-deletion gates and all file mappings
first, preserve the ledger and contract fixtures, perform the reviewed deletion,
then verify deletion and run the final completion check. Do not prematurely label
the deletion gate verified merely to make the pre-deletion command green.

## Audit validation

Read-only GitHub API inspection verified run conclusions, release assets and tag
target. `python3 scripts/hub-migration.py check` passed at the initial snapshot.
No workflow was dispatched, release changed, migration row certified or legacy
source removed by this audit. Documentation whitespace checks passed. Witness
selected no tests for the two markdown files; no runtime test coverage is claimed
for this documentation-only change. For current review counts, use
`python3 scripts/hub-migration.py backlog` (optionally `--json` or `--prefix PATH`).
