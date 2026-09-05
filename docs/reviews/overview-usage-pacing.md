# Overview usage pacing — independent review

Branch: `wks/workspacer-overview-usage-pacing`. Base: `43ba1dc022575e4f0c8a1639182fe1d68b4cb7b1` (v0.164.0 source). Delivery is a local commit for independent review; no merge, push, PR, release, or global configuration changes.

## Read first

- `services/hub/cmd/hub/usagereport.go` and `usagereport_test.go`
- `services/hub/internal/limits/report.go` and `report_test.go`
- `apps/desktop/src/main/shared/usageReport.ts`
- `apps/desktop/src/renderer/src/hooks/useUsageReport.ts`
- `apps/desktop/src/renderer/src/lib/usagePacing.ts`
- `apps/desktop/src/renderer/src/panes/OverviewPane.tsx` (`OverviewUsageCards`)
- `apps/desktop/src/renderer/src/components/UsageReportCard.tsx`
- `apps/desktop/src/main/ipc.ts`, renderer `backend/webBackend.ts`, `backend/bridgedBackend.ts`, and unchanged `backend/remoteBackend.ts`
- `apps/desktop/src/renderer/tests/usagePacing.test.ts`, `tests/components/usagePacingSurfaces.test.tsx`, and `tests/backend/backendParity.test.ts`

Task inputs read: AGENTS.md, CLAUDE.md, primary checkout `.workspacer/brief.md`, `.workspacer/reports/2026-09-04-usage-pacing-scout.md`, desktop DESIGN_LANGUAGE.md, and Rivet usage-accounting, limit-aware-routing, registration-checklists context. Rivet CLI used because MCP tools were unavailable; context recommendation used lexical ranking because the embedding service was unavailable.

## Acceptance criteria

- `usage.report` accepts no parameters, is available to view clients, and returns account provenance plus quota windows with optional sampled pacing. It produces no routing decision, log, event, provider catalog refresh, or provider launch.
- Per-window pace agrees with `PaceFor` under calendar, offset, workdays/timezone, reserve, disabled, and invalid configuration. Raw account/window fields remain readable by the old wire decoder. Known zero survives JSON serialization.
- Hub raw-cache age is at most 60 seconds; sampled pace validity is 60 seconds. Renderer shares a 60-second poll and a local one-second clock, removes verdicts at reset/deadline, marks retained observations after transport failures, and fences replies from replaced backends.
- Overview renders whole canonical report account observations, including cold starts. Conflicting live/federated readings do not supply their percentages or resets to a report row. Removed, unavailable, and rolled-over windows do not reappear from live caches.
- Comparison uses unrounded used minus expected: inclusive ±2 percentage points is “on pace”; greater than +2 is “ahead”; below −2 is “under.” Ahead has warning colour and “spending faster than expected” accessible text. Unknown, disabled, malformed, expired, and explicitly stale inputs show no pace verdict/tick. Zero consumes zero bar width.
- Default empty account key, unattributed null, distinct paths ending in `work`, and Windows account paths remain distinct. Full report identity appears in the detail dialog. Codex identifies its reported home, not an inferred login ID.
- Desktop IPC calls the hub first and falls back to the local raw daemon report. Bridged desktop delegates that IPC. Web and remote clients read their selected hub and return null for an unavailable/older hub, without consulting local IPC.
- Old daemon windows without lengths still show consumption without pace. Unavailable quotas have no phantom meter. Existing account detail click, Enter, Space, close, and Escape behavior remains available.

## Checks and runtime evidence

Gates were run sequentially. All final relevant checks passed:

- Hub: `go test ./internal/limits ./internal/routing ./internal/capspec ./internal/authtoken ./cmd/hub`; the same package set with `go vet` and `go test -race`.
- Desktop `npm run typecheck`, including final Node 22 run.
- Full main suite on Node 22: 148 files, 3,153 tests passed. Focused IPC/keep-warm regression: 55 tests passed.
- Full renderer suite on Node 22: 179 files, 1,692 tests passed. After the final Overview extraction/additional integration assertion, the focused renderer suite passed 96 tests across six files.
- Guard mutation: bypassing the `known:false` check caused the selector suite to fail at its verdict assertion; original code restored and focused suite passed.
- Changed TypeScript/TSX Prettier check, Go formatting, and `git diff --check` passed.
- Chromium: `npx playwright test --project=renderer usagePacing.test.ts` passed. Real card components rendered in light/dark themes at 360px and 1200px; no horizontal overflow; zero fill and keyboard detail identity asserted. Light/narrow and dark/wide screenshots were visually inspected.

Local browser captures: `apps/desktop/test-results/usagePacing-light-dark-nar-3c051-umption-and-keyboard-detail-renderer/usage-{light,dark}-{360,1200}.png`. Reproduce through the checked-in Playwright spec and `usage-pacing-harness.html`.

The initial broad run used shell-default Node 26 and failed 69 unrelated renderer tests because of experimental global localStorage; CI specifies Node 22, where the complete renderer suite passed. No load timeout needed classification or an isolated retry.

Runtime limits: HTTP handler tests used a real loopback HTTP server with fixture reports. Browser checks used the real components with fabricated observations. Transport mode compatibility used mocks. No live provider credentials, installed Electron app, or combined deployed hub/daemon stack was exercised. Rust, TUI, standalone mobile, provider probing, settings, release paths, and routing threshold changes are outside this slice.

## Changed files

- `.rivet/learnings/2026-09-04-overview-usage-projections-need-canonical-accoun-384d3b.md`
- `apps/desktop/playwright.config.ts`
- `apps/desktop/src/main/ipc.test.ts`
- `apps/desktop/src/main/ipc.ts`
- `apps/desktop/src/main/shared/usageReport.ts`
- `apps/desktop/src/renderer/src/backend/bridgedBackend.ts`
- `apps/desktop/src/renderer/src/backend/webBackend.ts`
- `apps/desktop/src/renderer/src/components/UsageReportCard.tsx`
- `apps/desktop/src/renderer/src/components/claude/UsageDetailDialog.tsx`
- `apps/desktop/src/renderer/src/harness/usagePacingHarness.tsx`
- `apps/desktop/src/renderer/src/hooks/useUsageReport.ts`
- `apps/desktop/src/renderer/src/lib/pluginPermissions.ts`
- `apps/desktop/src/renderer/src/lib/usagePacing.ts`
- `apps/desktop/src/renderer/src/panes/OverviewPane.tsx`
- `apps/desktop/src/renderer/tests/backend/backendParity.test.ts`
- `apps/desktop/src/renderer/tests/components/usagePacingSurfaces.test.tsx`
- `apps/desktop/src/renderer/tests/components/usageSurfaces.test.tsx`
- `apps/desktop/src/renderer/tests/pluginPermissions.test.ts`
- `apps/desktop/src/renderer/tests/usagePacing.test.ts`
- `apps/desktop/src/renderer/usage-pacing-harness.html`
- `apps/desktop/tests/e2e/usagePacing.test.ts`
- `docs/reviews/overview-usage-pacing.md`
- `services/hub/cmd/hub/main.go`
- `services/hub/cmd/hub/usagereport.go`
- `services/hub/cmd/hub/usagereport_test.go`
- `services/hub/internal/authtoken/authtoken.go`
- `services/hub/internal/capspec/capspec.go`
- `services/hub/internal/capspec/composition.go`
- `services/hub/internal/capspec/hubnative_test.go`
- `services/hub/internal/limits/report.go`
- `services/hub/internal/limits/report_test.go`
