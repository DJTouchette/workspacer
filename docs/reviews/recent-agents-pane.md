# Recent agents pane

Local implementation on `wks/workspacer-recent-agents-and-task-loops`, based on `e783759b`. No push, merge, live-primary install/build/restart, real worker launch, or daemon persistence change.

## Product and data contract

- Global singleton `recentagents` pane, opened from Fleet Deck or the command palette. Existing layout serialization/restoration retains it; reopening focuses it even in a split tab. It is absent from the default split menu.
- Defaults to the most recently started live local manager. Explicit all-locally-recorded-manager and project filters expose retained metadata, plus attempt-status and task-creation-time filters. Tasks are newest-created-first; expanded attempts retain acceptance order. There are no planned/skipped phase claims or automatic dispatch/retry/approval actions.
- Only accepted manager-owned desktop bus spawns are recorded. The facade derives `dispatchOwnerSessionId` from the request's session credential. The desktop checks it against the live local parent manager. Explicit `taskId` and `afterDispatchId` links are validated before launch; optional descriptive `stage` and automatic retry provenance may launch without a valid owner but do not record or link history. The returned host-generated task/dispatch IDs are consumed by later spawn calls and returned by respawn results only when recorded. Manager doctrine and MCP spawn help describe the continuation protocol.
- Task reuse requires the same owner and normalized project path, or an exact allocated execution path already recorded for that task. Parent IDs, routing roles, labels, and timing are never task identity. Omitted links/stage create standalone unclassified dispatches. No legacy session/transcript backfill.
- `respawn_with` supplies a private source field excluded from the public spawn input schema (`json:"-"`). The bus strips owner/source stamps from scoped operators, plugins, untrusted callers, and federation links; local host credentials remain trusted control-plane authority. The desktop verifies a known retry source's owner/project. Unknown legacy sources remain standalone and are not claimed as linked retries. Existing routing/fresh-role refusal, permission clamps, task cloning, role and result-schema inheritance remain intact.
- Private atomic `dispatch-history.json`, mode 0600, under the existing config directory. Retention: 200 tasks, 1,000 attempts, 2 MiB; eviction removes whole oldest tasks. Acceptance and validated results write immediately; telemetry writes coalesce at 500 ms; explicit close flushes. A crash can lose the last coalesced telemetry observation, not invent an outcome.
- Collection is independent of pane mounting. Session-store updates join only exact session IDs and replace cumulative observations, never sum replacement process generations. Ordinary resumed turns do not create attempts. Reload retains records as stale/last-observed and disables live links until an exact session is observed again.
- Lifecycle is independent of outcome. Only the existing `supervisorNudge` structured-result validator populates result-contract validity; prose, idle, and ended do not establish passing work. Escalation is separately labeled.
- Input/output/cache values are optional; unknown is not zero. Explicit reported zero survives, while empty usage-accumulator zero sentinels remain absent. Input includes cache tokens; cache tier splits appear only when available. Cost is **estimated API cost**, not subscription/invoice billing. Aggregate coverage is N/M and deduplicates exact dispatch IDs; retries count as distinct attempts within one task. Wall time spans accepted dispatch to observed end or now for live attempts; stale rows retain the last observation. Context is a dated point-in-time snapshot. Provider is labeled requested/resolved; model requested and latest reported values are separate, with no per-token historic model attribution.
- Open agent rechecks the exact live local session before using the existing viewer event, including its provider. Review uses existing inline `FleetReview` and opaque evidence IDs. Its owner comes from the host query's current manager, never the row's owner field. Other/stopped managers do not acquire review access from local history. History review additionally rechecks the live owner at click time. Existing review-store owner/selector validation, revocation/eviction behavior, and filesystem-root grants are unchanged.
- Read surface is optional preload IPC plus `HOST_ONLY` bridged delegation. Web/remote and absent old preload return explicit unavailable. No new bus read capability, headless registration, remote collector, filesystem capability, or generic delegation was added. Brain spawn-field parity explicitly records the desktop-only fields as declined. Sessions/Analytics remain separate and unchanged.

## Reviewer entry points

1. `apps/desktop/src/main/services/dispatchHistoryStore.ts` and `shared/dispatchHistory.ts`: persistence, identity, retention, lifecycle and metrics.
2. `apps/desktop/src/main/services/hubCapabilities.ts`, `services/hub/cmd/mcp/main.go`, `respawn.go`, and `services/hub/internal/bus/rpc.go`: accepted-spawn IDs, public continuation params, private owner/retry stamps, and trust boundary.
3. `claudeSessionStore.ts` and `supervisorNudge.ts`: small lifecycle and existing-validator adapters.
4. `apps/desktop/src/renderer/src/panes/RecentAgentsPane.tsx`, `components/claude/FleetReview.tsx`, and `backend/bridgedBackend.ts`: actual pane, current-owner review gate, and local backend routing.
5. IPC/channels/preload/types; pane/icon/lazy-switch/opener/palette/Fleet Deck/title registrations; manager doctrine/help; focused tests, Chromium fixture, and capspec field vocabulary/parity guards.

## Validation

Environment: isolated supplied worktree; Node 22.22.2; Go 1.25.4; Chromium 148.0.7778.96. Desktop and renderer dependencies were installed privately from their existing lockfiles using `npm ci --ignore-scripts`. Lockfiles and shared/live dependency trees were not changed. Test batches ran sequentially.

- Main and renderer TypeScript checks: passed.
- Focused main tests: 368 passed in seven files (history store, actual spawn handlers, session lifecycle, validator adapter, existing review store, IPC, preload). History tests use real temporary persistence; provider launch is mocked. Includes manager-only admission, owner/project/predecessor refusal, actual stages, host-returned ID reuse, worktree metadata, failed-launch exclusion, retries/resumes, close/restart, stale reload, retention and missing metrics.
- Focused renderer tests: 63 passed in six files (metrics coverage, singleton restore/open, backend parity, existing FleetReview, pane menu, manager doctrine).
- Nine Chromium production-pane tests passed using the actual bridged factory with mocked IPC and bus snapshots: 360/1280 widths, dark/light themes, keyboard expansion, long loops, combined filters, partial metrics, foreign-owner review exclusion, eviction errors, empty/unsupported/stopped states, exact live Open agent, and stopped-owner review refusal. Screenshots are generated under `apps/desktop/test-results/` by the reproducible fixture.
- Go MCP, brain, bus, and focused capspec tests: passed. The full capspec run has two unrelated scanner failures on unchanged source: `TestEveryClaudemonCallerPathIsServed` finds zero callback paths in `services/claudemon/src/providers/codex.rs` against its floor of one; `TestEveryClaudemonCallerFileIsEnumerated` reports the existing `htmlCardHarness.tsx` listener literal as unclassified. The focused run excludes only those two tests; all new spawn-field vocabulary, floor, and provenance guards pass.
- Changed-file Prettier, gofmt and `git diff --check`: checked before commit.

Reproduce from this worktree with Node 22 on PATH:

```sh
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run test:main -- src/main/services/dispatchHistoryStore.test.ts src/main/services/claudeSessionStore.test.ts src/main/services/supervisorNudge.test.ts src/main/services/hubCapabilities.test.ts src/main/services/fleetReviewStore.test.ts src/main/ipc.test.ts src/main/preload.test.ts
npm --prefix apps/desktop run test:renderer -- tests/RecentAgentsPane.test.tsx tests/openPaneInDedupe.test.tsx tests/backend/backendParity.test.ts tests/fleetReview.test.tsx tests/paneMenu.test.ts tests/fleetManager.test.tsx
npm --prefix apps/desktop exec -- playwright test --config=apps/desktop/playwright.config.ts --project=renderer recentAgents.test.ts --workers=1
(cd services/hub && go test ./cmd/mcp ./cmd/brain ./internal/bus ./internal/capspec -skip 'TestEveryClaudemonCaller(PathIsServed|FileIsEnumerated)')
```

No Electron live-app restart or real provider execution was performed. Full capspec scanner cleanup and a live-provider smoke test remain separate follow-ups for the manager/reviewer.

## Follow-up repair

Commit pending: optional stages and automatic respawn provenance no longer refuse an otherwise valid launch when manager attribution is absent or has changed. Explicit task/predecessor links remain pre-launch owner/project checks. Focused store and real desktop spawn-handler tests cover unowned stage, non-manager parent, handoff/unknown retry sources, owned continuations, and forged/foreign explicit links; MCP respawn output omits absent history IDs. The focused handler test uses mocked provider launch and direct capability registration; it does not yet exercise a facade call through a live hub into the desktop provider.
