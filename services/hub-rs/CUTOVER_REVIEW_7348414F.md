# Cutover gate review at 7348414f

Read-only evidence audit, 2026-09-30. Candidate is
`7348414f3d63f88faf98bf156f7e015776690dcb`. This preparation document neither
certifies nor edits the main ledger. Its14 gates are still pending; the older
`cutover-evidence-plan.json` describes a265b956 and must not be relabeled as this
candidate. The source review count is separate from these gates.

## Exact execution receipts

- [Primary36669665085](https://github.com/DJTouchette/workspacer/actions/runs/36669665085): all9 jobs succeeded, including desktop/browser integration, Windows containment, source/generator guards, hub latency, daemon and TUI. Hub job109741742819 explicitly executed `real_rust_backend_calls_events_reconnect_and_owned_shutdown`:1 passed,0 ignored.
- [Native36669665067](https://github.com/DJTouchette/workspacer/actions/runs/36669665067): all3 OS jobs succeeded. Linux real-window and rich-transcript smoke executed; equivalent GUI smoke is not claimed for Windows/macOS.
- [Preview36669665055](https://github.com/DJTouchette/workspacer/actions/runs/36669665055): Linux/macOS services and Windows preview succeeded. Windows job109741742150 executed install, upgrade, embedded backend, joined shutdown and uninstall; its receipt has `connected:true`, `ports_released:true`, `checked_ports:4`, `shutdown_joined:true`. This is the preview/backend harness, not a visual installed GPUI session or production identity upgrade.
- There is no release or container run at this exact SHA in the queried run listing.

## Fourteen-gate disposition

| Gate | Honest disposition now | Smallest remaining action / claim limit |
| --- | --- | --- |
| `ci-and-generators` | Ready for owner certification at this SHA. | Attach primary job receipts. Existing source guards still intentionally use retained Go bytes on main; the separate deletion transition is not certified by this gate. |
| `tui` | Ready for owner certification at this SHA. | Attach normal TUI job plus the explicitly executed hub-owned reconnect/shutdown test above. |
| `native-embedded` | Ready for owner certification within supported preview scope. | Attach native3-OS and Windows preview installation/embedded/shutdown receipts. Do not claim installed GUI interaction on all OSes or production-state upgrade from an isolated preview identity. |
| `federation-and-remote-workers` | Behavior can be certified with an explicit source-equivalence review. | Exact candidate hub integration tests pass. Older container worker registration/boot receipt can be carried only with unchanged-input scope below, not renamed as a734 artifact. If policy requires literal same-SHA container evidence, obtain that run. |
| `deployment` | Final-head receipt still required by the ledger's existing explicit requirement. | Run the container contract on the selected final SHA, or deliberately document an owner-approved change to that receipt requirement using the source-equivalence review. Existing fresh-volume boot does not prove cloud rollout or all existing-volume upgrades. |
| `electron-and-web` | Not ready: a relevant packaged execution is failing. | Resolve and re-execute actual packaged Linux Electron owned/adopted ownership smoke. Browser CI does not replace this. |
| `standalone-service` | Runtime ready; final package receipt incomplete. | Obtain all3 extracted standalone bundle readiness/catalog/shutdown receipts for final release. Prior Win/Mac receipts are useful unchanged-source evidence, but Linux release smoke was skipped after the Electron failure. |
| `mcp-and-plugins` | Runtime ready; final packaged provenance remains to attach. | Exact hub/desktop integration passes. Win/Mac4fd packaged smoke verified modern+legacy catalogs; retain those as their actual SHA, or attach final release3-platform receipts. Do not lower the failed Electron catalog floor to create a receipt. |
| `persisted-state-upgrade` | Requires owner review of the bounded inventory/claim. | Current owning reopen/recovery tests pass; see `PERSISTED_HISTORY_FINDINGS.md`. Either certify only the documented supported representations and executed restart/error contracts, or add the3 independent TS-written task/review/replacement import controls before claiming cross-writer historical import. Fresh install/preview is insufficient. |
| `node-companion-replacement` | Implementation/source review ready; package evidence incomplete. | Attach final no-Node standalone and native payload/harness receipts and update stale operational docs. Optional plugin/provider Node runtimes are outside the retired private companion. Unmerged prep portability changes cannot certify main. |
| `windows-package` | Not ready for final integrated release certification. |734 native preview passes;4fd Electron/server job passed, but that release's native package and publication jobs were skipped. Obtain final native/backend CRT/provenance handoff, native installation and Electron/update assets at chosen release SHA. |
| `macos-package` | Prior package/runtime scope passed; exact final artifact remains. |4fd DMG/server job passed and734 macOS runtime passed. Obtain chosen final release DMG/server stamps and smoke. No native macOS installer or automatic Electron updater requirement should be invented. |
| `linux-package` | Not ready: packaged Electron ownership failure. | Fix/re-run that smoke, then finish the skipped standalone smoke and final AppImage/server artifact receipt. |
| `legacy-deletion` | Not ready and not authorized by this audit. | Integrate/review prep portability and reference-command changes, finish other gates, preserve retained assets, then separately authorize actual removal and run absence/dependency/ready checks. Go is still present on main. |

## Relevant older receipts and source equivalence

[Release36668076976](https://github.com/DJTouchette/workspacer/actions/runs/36668076976)
was built from `4fd429abb3bb2b12a5b1ee0abee16bf188d203ec` and **failed**.
Windows job109736984247 and macOS job109736984327 succeeded, including extracted
standalone smoke with no Node on PATH. Windows emitted modern/legacy protocol
versions2026-07-28 and2025-11-25, authenticated brain probe, four listeners and
joined shutdown. Linux job109736984355 failed with:
`adopted Electron smoke failed: operator catalog below retained100-tool floor`.
Its later standalone steps, native-windows-package and publish-nightly were
skipped. Those skipped jobs cannot inherit Windows native-build success.
The observation establishes a failing assertion, not whether its cause is a
provider defect or readiness timing; root owns that diagnosis.

[Container36664561953](https://github.com/DJTouchette/workspacer/actions/runs/36664561953)
succeeded at `a265b956fb5ba8fa2b7c8f92210d52e16f578f5c`. Comparing that commit with
734 yields no changes in `services/hub-rs`, `services/claudemon`, `deploy/fly`,
`contracts`, renderer sources, `vendor/portable-pty`, or the container workflow.
The intervening changes are the Electron smoke correction and native GPUI/UI,
font, vendor-component, native manifest/lock and documentation changes. Thus the
prior container test's backend/deployment input scope is unchanged. Its image
source stamp nevertheless remains a265, and the existing ledger specifically
asks for a final-head deployment rerun. Scope equivalence is a review decision,
not an assertion that an absent workflow ran.

## Priorities

1. Resolve the packaged Electron catalog-floor failure; re-run release at the
   chosen integrated SHA. This unblocks the Linux, Electron, standalone and final
   cross-platform package/provenance receipts together. Do not dispatch or publish
   merely because this audit names the needed execution.
2. Record the3 directly supported gate decisions now if root accepts their stated
   scope; separately review federation carry-forward and schedule the final-head
   container receipt already required by deployment.
3. Decide the persisted-state claim precisely and add only the3 missing provenance
   controls if independent TS-origin import is part of that claim.
4. Keep deletion last. The ready prep provenance/wrapper changes are not in734 and
   no source removal or `ready` success has occurred.

Evidence logs inspected locally: `/tmp/workspacer-gate-hub-734.log`,
`/tmp/workspacer-gate-windows-preview-734.log`,
`/tmp/workspacer-gate-release-{windows,macos}-4fd.log`, and
`/tmp/workspacer-gate-electron-failure-4fd.log`. GitHub URLs above are the durable
receipts; local files only aid inspection. No builds or workflow actions ran.
