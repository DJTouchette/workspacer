# Workspacer Rivet documentation audit

Status: **completed source audit for this checkout, with explicit validation limits**.
Scope confirmed by the user: Workspacer, not Preheat or Rivet upstream.
Audit started 2026-09-26. The checkout and executable source are authoritative;
historical comments and learning entries are hypotheses to verify.

## Completion requirements

- Review every curated context document and operational runbook against current
  implementation; resolve contradictions and retain useful verified invariants.
- Check coverage against current subsystems and recent learnings; add missing
  guides and promote findings without rewriting historical entries as current facts.
- Validate local source paths, links, metadata, retrieval, and relevant documented
  commands/examples. Record platform or external-service limits explicitly.
- Regenerate discovery instructions through Rivet; preserve hand-authored text.
- Verify the final diff and report remaining limitations. Clean lint alone is
  insufficient to complete this goal.

## Repeated checks

From the repository root:

```bash
rivet doctor
rivet context lint
python3 scripts/check-rivet-docs.py
git diff --check
```

The new checker validates related-path globs, explicit root-relative source
paths, and inline local links/anchors. It excludes historical learnings, external
URLs, extensionless build artifacts and ambiguous code symbols. It is not a full
Markdown parser or semantic verifier. Temporary fixtures confirmed it rejects
missing files, missing HTML anchors and unmatched globs while ignoring fenced
examples. Rivet owns YAML validation.

`AGENTS.md` is a tracked symlink to `CLAUDE.md`. Discovery guidance was
regenerated with `rivet sync --provider claude` so the shared target retains its
Claude project-agent guidance. Running `--provider both` writes this same file
twice; its final Codex pass removes that provider-specific line. Generated
sections were not hand-edited. The new companion guide is present in discovery
and ranks first for the matching lexical recommendation query.

## Review ledger

“Reviewed” means the rewritten guide's current claims were checked against named
source and the listed focused validation. “Partial” means specific fixes have
evidence, but the rest of the document has not been certified. “Pending” means
only corpus-wide structural/reference checks have run.

| Document | Status | Evidence / remaining work |
| --- | --- | --- |
| [domains/agent-spawn.md](../../.rivet/context/domains/agent-spawn.md) | Reviewed | Completed launcher/peer payload, prefill versus first-message, cwd and facade review. Existing spawn/skill tests plus 6 concurrent worktree-admission tests passed; provider readiness is explicitly distinct. |
| [domains/auto-update-release-channel.md](../../.rivet/context/domains/auto-update-release-channel.md) | Reviewed | Completed updater and release workflow review; removed false atomic rollout and macOS error-path claims. Prior 39 updater tests passed; no publication/install attempted. |
| [domains/chat-tool-rendering.md](../../.rivet/context/domains/chat-tool-rendering.md) | Reviewed | Completed current rendering, send identity, skill/image, timer and scroll contracts. Prior 65 chat/cache/model-switch tests and 39 git/file/path tests passed; no new geometry guarantee. |
| [domains/claudemon-http-api.md](../../.rivet/context/domains/claudemon-http-api.md) | Reviewed | Completed hook/init/ask/oneshot review, fixed displaced-card restoration and route contract reference. Added 14 ask, 13 init and 1 fake-CLI oneshot tests to prior API/hook/wrapper validation. |
| [domains/config.md](../../.rivet/context/domains/config.md) | Reviewed | Consolidated writer, lock/stamp, wholesale-map, presence-aware merge, failure and state-loss contracts. Existing 83 TS and Go Config/Wholesale checks support the reviewed source. |
| [domains/cross-provider-handoff.md](../../.rivet/context/domains/cross-provider-handoff.md) | Reviewed | Completed Rust builder, TS and Go rich tier, desktop dialog and TUI review. 9 dialog tests and focused Go handoff/library checks passed; prior 4 Rust builder checks retained. |
| [domains/desktop-remote-client-mode.md](../../.rivet/context/domains/desktop-remote-client-mode.md) | Reviewed | Rewritten against current tabs, workers/client ownership, native controls and local onboarding dismissal. Prior 10 persistence/5 selection checks and current renderer remote-node tests passed. |
| [domains/mission-control-attention.md](../../.rivet/context/domains/mission-control-attention.md) | Reviewed | Completed signatures, suppression limits, dock/triage, timer and bounded notification-history review. 60 resolver/notification/attention tests passed; prior projection/action-isolation checks retained. |
| [domains/remote-mobile.md](../../.rivet/context/domains/remote-mobile.md) | Reviewed | Completed shell/auth, UI/federation, conversation, service-worker and push review. 35 real-hub/fake-provider Chromium tests passed after temporary library setup; initial launch failure retained below. |
| [domains/renderer-backend-seam.md](../../.rivet/context/domains/renderer-backend-seam.md) | Reviewed | Consolidated current delta push/fallback, shared-service drill-in, sparse aliases and transport contracts. 100 renderer backend/federation/node tests and focused Go conversation checks passed. |
| [domains/session-lifecycle.md](../../.rivet/context/domains/session-lifecycle.md) | Reviewed | Consolidated current generation, pending-owner, subagent, liveness, history and journal contracts. 366 daemon session tests (1 ignored), 92 TS store/pending tests and 29 renderer history/stall tests passed. |
| [domains/tui-client.md](../../.rivet/context/domains/tui-client.md) | Reviewed | Completed bootstrap, raw TCP/direct dependencies, Driver/federation and render-budget review. Full TUI suite passed: 451 tests, 2 ignored. |
| [domains/usage-accounting.md](../../.rivet/context/domains/usage-accounting.md) | Reviewed | Consolidated pricing, TTL/cache wire, runtime health, session-free account reports, polling and persistent headless analytics. 87 renderer usage tests plus Go limits/hub projection checks passed; daemon session suite covers account/report code. |
| [modules/claude-asset-roots.md](../../.rivet/context/modules/claude-asset-roots.md) | Reviewed | Completed Rust/TS/Go inventory/library root and mutation review; documented profile/discovery-cap limits. 74 TS library tests and focused Go library tests passed; prior Rust provider suite covers enrichment. |
| [modules/claudemon-providers.md](../../.rivet/context/modules/claudemon-providers.md) | Reviewed | Rewritten around current admission, transports, controls, protocol joins and failure semantics; prior 258 adapter/8 engine tests plus current managed-spawn suite support the guide. Live external CLIs remain explicitly outside that evidence. |
| [modules/claudemon-pty-wrapper.md](../../.rivet/context/modules/claudemon-pty-wrapper.md) | Reviewed | Wrapper/PTY/endpoint source checked; corrected signal helper, frame handling and registration identity, with shutdown limits explicit. Five wrapper endpoint tests passed. |
| [modules/claudemon-sqlite-store.md](../../.rivet/context/modules/claudemon-sqlite-store.md) | Reviewed | Consolidated field-specific upsert, hydration limits, separate leases, retention and v8 migration contract. 136 store tests passed; documented lack of provider in RestoredSession. |
| [modules/claudemon-watch-tui.md](../../.rivet/context/modules/claudemon-watch-tui.md) | Reviewed | Completed embedded UI/source review; corrected gate mirror, imported model and idle-timer claims. 47 Rust TUI tests passed. |
| [modules/filelink-openable-files.md](../../.rivet/context/modules/filelink-openable-files.md) | Reviewed | Completed path detector, render-time cwd, exact preview dedup, editor/browser and feedback review; prior 39 renderer git/file/path tests support the updated guide. |
| [modules/fleet-manager.md](../../.rivet/context/modules/fleet-manager.md) | Reviewed | Replaced obsolete isSupervisor/manager-only wake and memory-only succession claims; current doctrine, skills, progress/threshold, journal and wake source checked. 271 focused TS tests and Go fleet/manager checks passed. |
| [modules/fly-node-deploy.md](../../.rivet/context/modules/fly-node-deploy.md) | Reviewed | Completed topology, credentials, base-image and build-stamp review; 51 offline release fixtures and shell syntax checks passed. No live deployment. |
| [modules/git-review.md](../../.rivet/context/modules/git-review.md) | Reviewed | Desktop/brain repository rooting, mutation ownership, parser and containment contracts reviewed; TS/renderer and focused Go git tests passed. |
| [modules/headless-desktop-services.md](../../.rivet/context/modules/headless-desktop-services.md) | Reviewed | New coverage for shared Node companion, registry/generator and authority; production-protocol Node and Go integration passed. |
| [modules/hub-bus-control-plane.md](../../.rivet/context/modules/hub-bus-control-plane.md) | Reviewed | Caller gates, owner provenance, canonical/object containment, RPC ownership and federation reviewed; bus/capspec tests passed on isolated rerun, initial latency failure retained below. |
| [modules/hub-federation.md](../../.rivet/context/modules/hub-federation.md) | Reviewed | Completed forwarding, peer replacement, tombstones, client transcript and model-routing review; 45 desktop and 23 TUI federation tests passed. |
| [modules/hub-jobs.md](../../.rivet/context/modules/hub-jobs.md) | Reviewed | Handler ownership, scheduler, context guards, persistence, CLI and facade; focused Go suites passed. |
| [modules/hub-plugin-system.md](../../.rivet/context/modules/hub-plugin-system.md) | Reviewed | Manifest namespace validation, ambient compatibility metadata, token lifecycle, settings redaction and catalog verified; plugin suite passed. |
| [modules/hub-process-supervision.md](../../.rivet/context/modules/hub-process-supervision.md) | Reviewed | Rewritten against Unix/Windows watcher and supervisor code, with cleanup/failure limits explicit; Windows packages cross-compiled successfully (not runtime-tested). Linux/CLI supervisor evidence retained. |
| [modules/hub-shared-cap-event-vocabulary.md](../../.rivet/context/modules/hub-shared-cap-event-vocabulary.md) | Reviewed | Corrected active EventGrants.Provides boundary; registration/vocabulary source and bus/capspec suite verified. |
| [modules/hub-web-push.md](../../.rivet/context/modules/hub-web-push.md) | Reviewed | Trigger, preference, revocation, key-loss, endpoint and delivery source; push suite passed. |
| [modules/ipc-boundary.md](../../.rivet/context/modules/ipc-boundary.md) | Reviewed | Finished type/optional-method, return-shape, window/port ownership, snapshot and validation review; prior 47 IPC/preload tests retained as evidence. |
| [modules/limit-aware-routing.md](../../.rivet/context/modules/limit-aware-routing.md) | Reviewed | Completed advisory selection versus composed dispatch and platform logging review; MCP routing/dispatch tests and Windows routing cross-compile passed. |
| [modules/mcp-tool-facade.md](../../.rivet/context/modules/mcp-tool-facade.md) | Reviewed | Reviewed conditional injection/readiness, identity, ambient plugin catalog and generation-fenced supervision; focused Go and 18 TS facade checks passed. |
| [modules/pane-system.md](../../.rivet/context/modules/pane-system.md) | Reviewed | Pane union, rendering switch, menus, error boundary, and geometry; 33 focused renderer tests passed. |
| [modules/theme-system.md](../../.rivet/context/modules/theme-system.md) | Reviewed | Resolver/defaults, editor persistence, token projection and guest filtering checked; 10 theme tests passed. |
| [modules/ui-modes-manifest.md](../../.rivet/context/modules/ui-modes-manifest.md) | Reviewed | Checked actual manifest/sidebar/App consumers, removed obsolete history-as-current guidance and clarified rejected versus unchanged saves; prior UI/sidebar tests passed. |
| [modules/webview-security-hardening.md](../../.rivet/context/modules/webview-security-hardening.md) | Reviewed | Rewritten against current file/attach/navigation/popup and plugin iframe policy; 91 main policy and 13 renderer fallback tests passed. Platform integration limits stated. |
| [modules/workflow-subagent-watcher.md](../../.rivet/context/modules/workflow-subagent-watcher.md) | Reviewed | Completed artifact/final adoption, output-slice, cache, synchronous parse and native-provider routing review; existing watcher attribution tests passed. |
| [modules/worktree-artifact-cleanup.md](../../.rivet/context/modules/worktree-artifact-cleanup.md) | Reviewed | New scheduler/core/admission coverage; 19 core/scheduler, 4 CLI and 6 Rust admission tests passed using disposable fixtures. |
| [modules/workspacer-serve-cli.md](../../.rivet/context/modules/workspacer-serve-cli.md) | Reviewed | Completed CLI/token/status and launcher lifecycle review; full cmd/workspacer suite passed. Corrected brain.info status and warning-only init preflight. |
| [paradigms/architecture-overview.md](../../.rivet/context/paradigms/architecture-overview.md) | Reviewed | Reviewed current process ownership, launch order, queueing, state/transport and TUI token behavior; linked verified subsystem contracts. |
| [paradigms/hotspots.md](../../.rivet/context/paradigms/hotspots.md) | Reviewed | Replaced stale numbers with current commands and source-backed cluster guidance; current hotspots inspected. |
| [paradigms/registration-checklists.md](../../.rivet/context/paradigms/registration-checklists.md) | Reviewed | Contract/authority guidance checked against current registries; added shared-service manifest/generation checklist and verified integration command. |
| [paradigms/renderer-event-buses.md](../../.rivet/context/paradigms/renderer-event-buses.md) | Reviewed | Completed producer/consumer and mount/target contracts; corrected cwd reuse, async publication limits and watch versus composer consumers. Source review; no cold-lazy-mount browser guarantee. |
| [paradigms/renderer-live-state-hooks.md](../../.rivet/context/paradigms/renderer-live-state-hooks.md) | Reviewed | Corrected workspace failure warnings/retry hash, restore fencing, snapshot-hook ownership, desktop reconnect and normalized layout echo handling. 30 layout/session hook tests passed. |

## Runbooks

Both embedding runbooks were corrected for Workspacer's ignored-cache policy,
index errors, model identity, and separate source-checkout requirements. The
ONNX binding is now pinned to v1.31.0 to match Runtime 1.26.0. That pair is
confirmed by the binding's versioned README.

An isolated Linux x64 test built Rivet v0.20.0 with that binding, downloaded the
runtime/model, indexed the then-current 45-document corpus (227 vectors), ran
indexing again (zero new chunks), and confirmed `semantic-match` retrieval.
The headless companion and cleanup guides increase the final curated corpus to 47 documents.
macOS and HTTP backend setup were not executed; the runbooks state the test scope.
No installed Rivet binary or configured user backend was replaced. Temporary
verification data lives outside the repository.

## Validation evidence and limits

- Go: brain Config/Wholesale passed; routing initially failed because an append-test fixture assumed umask 0022 while the session uses 0077. A separate-shell run with umask 0022 passed; production source was unchanged. TS config service tests (83) passed.
- Go: a broader bus run failed the snapshot latency benchmark (10.14 ms incremental p99 versus 5 ms budget); an isolated full bus rerun passed. Both outcomes are retained rather than treating the first run as green. Capspec passed.
- Go: jobs, claudemon bridge, plugins, push, supervisor, federation, focused hub
  owner/peer-config tests, focused CLI tests, and MCP tier/job selection passed.
- Rust: provider adapters (258 passed, 3 ignored tests; no live-provider validation claimed) and execution registry (8) passed. Pricing/TTL checks (11) passed.
- Rust: schema migration (10), retention prune (3), and stale-stopped eviction (1) tests passed.
  `cargo test --lib daemon::api::tests` (106) and
  `cargo test --lib daemon::hook::tests` (5) passed. An initial `--bin` invocation
  ran zero tests and is not counted as validation.
- Desktop: model usage/pricing contract tests (96) passed.
- Desktop: git service and workflow-watcher ID tests (18) passed; renderer git/file-link/path-detection tests (39) passed.
- Desktop: preload/IPC tests (47) and manager replacement/reconciliation tests (43) passed.
- Generated ordinary-agent assets and 11 installer tests passed; four Rust mechanical-handoff and focused Go first-message/collaboration checks passed.
- Webview guards/roots (91) and guest iframe fallback (13) passed. Facade supervision/config (18) and focused brain facade checks passed.
- Renderer chat/cache/model-switch tests (65), remote-mode selection (5), and remote-server persistence tests (10) passed.
- TUI: seven model-switch contract tests passed. Update service: 39 tests passed. Attention projection/action isolation: 17 tests passed.
- Renderer: pane menu/layout/palette (33), UI-mode/sidebar (9), and theme (10) tests passed. A supplied
  `configContext.test.tsx` filter matched no file and provides no config coverage.
- `npm run test:desktop-host` passed two Node protocol/analytics tests and both
  Go companion/replacement integration tests with the built bundle supplied.
- `internal/parentwatch` has no test files; a successful package check is not a
  platform-specific watchdog test.
- No real push notifications, deployment, publication, or production restarts
  were performed. Cross-platform/browser behavior not covered above still needs
  appropriate runtime verification before broader platform/service claims.

## Latest structural checkpoint

47 curated documents (45 context guides and 2 runbooks) pass Rivet lint.
All 45 context guides are marked Reviewed; no Partial or Pending rows remain. Both runbooks have a scoped Linux verification record.
The local-reference checker is clean;
`git diff --check` is clean. The reference count changes as guides are revised.
These totals describe structural validation, not the number of fully reviewed
guides. The review ledger above remains the completion authority.

## Ongoing maintenance

Maintain this baseline when implementation changes: update affected guide claims,
run relevant checks, and distinguish new learning observations from verified
current contracts. Application defects identified during documentation review
are recorded as limits, not silently fixed in this documentation-only change.

## Second-pass evidence (2026-09-26)

Session/provider/usage/fleet and SQLite/API guides now have consolidated current
contracts instead of contradictory append-only historical instructions. Exact
new checks and counts are recorded in their ledger rows above. These are source
and controlled-test reviews, not fresh claims about live provider versions,
production accounts, or untested operating systems. The remaining guide reviews are now complete; later evidence below covers
the final renderer/mobile/TUI and cleanup pass.

## Final-pass evidence and known implementation limits

- Mobile: the first 35-test attempt failed before assertions because Chromium
  lacked Linux libraries (`libatk-1.0.so.0`, among others). Debian packages were
  downloaded and extracted under `/tmp/workspacer-rivet-browser-libs`, without
  system installation or repository dependency changes. With that directory on
  `LD_LIBRARY_PATH`, all 35 mobile tests passed in 19 seconds. These tests use a
  real freshly built hub and fake session provider; no live LLM/account is used.
- The final guide review adds 45 desktop federation, 23 focused TUI federation,
  100 renderer backend/federation/node, 60 attention/notification/resolve, 9
  handoff-dialog and 74 library tests. The full TUI run supersedes the focused
  TUI count: 451 passed, 2 ignored. Do not add overlapping counts as independent
  coverage. Full CLI and focused Go handoff/library/conversation/routing checks
  passed. Windows routing and supervision packages were cross-compiled only.
- Deployment: 51 offline release-fetch fixtures and shell syntax checks passed.
  No cloud deployment, release publication or installer application occurred.
- Cleanup: 19 core/scheduler tests, 4 CLI tests and 6 Rust admission tests passed.
  An initial test command named a nonexistent main-side compaction test; it ran
  only the two actual cleanup suites and is not counted as compaction coverage.
- Newer learning titles and relevant observations were reviewed for coverage.
  Added the missing cleanup/admission guide, promoted its verified learning
  records, and captured source discrepancies separately. Historical learning
  prose remains historical; it is not all certified as current documentation.

Current application limits explicitly retained in the guides include web peer
seeding that skips sparse rows, mobile sequence anchoring that differs from the
renderer coalescing-safe fetch, colon-sensitive attention suppression pruning,
embedded-watch gate drift/idle-timer behavior, and the nightly deletion-to-publish
failure window. Stable Windows metadata must match actual uploaded asset names;
local updater code does not repair dotted/dashed mismatches. These findings do
not make stronger UI/runtime guarantees true merely because fixture tests pass.

The audit establishes a reviewed documentation baseline for this checkout. It
does not promise permanent perfection, live provider compatibility, real push
delivery or runtime validation on untested operating systems.

## Final retrieval and consistency checkpoint

All 47 curated documents pass metadata/retrieval lint; 630 local references pass
the repository checker. Rivet doctor reports no errors, with optional semantic
configuration absent in the user's normal environment. Generated discovery was
refreshed with `--provider claude`; the tracked AGENTS symlink and hand-authored
CLAUDE footer are preserved. No application source was changed.

Lexical queries rank the new headless-service and cleanup guides first for
their matching tasks, and mobile first for PWA/offline/push. An isolated ONNX
copy of the final 47-doc corpus indexed successfully; after updating changed
chunks its local cache contains 182 vectors (including earlier content keys).
The reconnect query ranks session-lifecycle first with `semantic-match`. This
uses the runbooks' pinned Linux toolchain without replacing the installed Rivet
or configuring the user's backend. The final unchanged index adds zero chunks.

The three correctly located renderer compaction/snapshot-fold test files also
passed (31 tests), covering the newly promoted compaction guidance. No remaining
Partial/Pending ledger rows or known broken local references remain.
