# Restore Overview navigation

Base: `e443270bd6a493ab60eece721d53de2405e6a11d` (released 0.165.0).
Local branch: `wks/workspacer-restore-overview-access`.

Overview was still present: `PaneType`, `PaneIcon`, default titles and
`ScrollContainer`'s lazy `OverviewPane` registration all remained intact. Its
existing tab could be clicked in the global workspace's tab bar. The sidebar
brand and collapsed Overview tile, and command-layer `go-overview` (`prefix 0`,
`:dash` / `:home` / `:overview`), only selected the global workspace. They neither
selected the dashboard tab nor scrolled to it. Opening Recent agents selected
another tab in that workspace; both home navigation and layout restore retained
that selection. Closing Overview while another global tab remained also bypassed
the empty-workspace fallback.

The expanded sidebar row disappeared in `e005985c` (2026-07-18). `4aad7498` added
Recent agents without removing Overview. `2ba8d92e` put Recent agents beneath the
Fleet runbook timeline. The regular palette and the new-tab menu had no Overview
entry. Focus gates the Fleet overlay, not the dashboard; restored Fleet altitude
can cover the underlying global tabs.

Users now have a labeled Overview button below the sidebar brand and beside
Recent agents in the Fleet timeline, plus **Open Overview** in the regular
palette (`Ctrl/Cmd+K`). These, the brand, collapsed tile and existing command-layer
action share `App.openOverview`: set piloting altitude, open/focus Overview in
global, then scroll to its tab. UI mode remains the user's selection. Existing
renamed/split Overview panes are reused; a closed dashboard is recreated.

Reviewer entry points:

- `apps/desktop/src/renderer/src/App.tsx`: shared route and all callback wiring.
- `apps/desktop/src/renderer/src/hooks/useAgentManager.ts`: singleton pane reuse.
- `SideBar.tsx`, `CommandPalette.tsx`, `FleetDeck.tsx` in renderer components:
  visible entries. Fleet's ordering, manager anchor, real chat/Back and retained
  pane ownership are unchanged. No duplicate dashboard, backend or settings schema.
- `apps/desktop/src/renderer/tests/openPaneInDedupe.test.tsx` and the Overview
  navigation cases in `apps/desktop/tests/e2e/firstUse.test.ts`: restored selection,
  closed/renamed/split dashboard, Focus/Fleet, sidebar/rail/brand/palette/leader.

Environment: Linux, Node 22.22.2, private desktop and renderer dependencies from
the existing lockfiles (`npm ci --ignore-scripts`); every heavy check runs
sequentially through `scripts/release-check.py` with its default 2 GiB cap.
Browser tests use synthetic IPC and an ephemeral Vite server. They do not verify
Windows Electron or contact live app/provider state. No new release or live restart.

Validation (all passed): desktop `npm run typecheck` (main + renderer); focused
renderer Vitest selection (9 files, 99 tests); Playwright renderer
`firstUse.test.ts --grep 'Overview navigation'` (4 restored-mode cases);
`fleetContextMenu.test.ts recentAgents.test.ts` (21 cases, including 360/1280 px,
light/dark, timeline manager position and retained draft/iframe/Back behavior).
Browser version: Chromium 148.0.7778.96. Changed TypeScript files pass Prettier.
Test workers were limited to one. Initial new browser-test runs failed because
the test used the unresolved Linux leader and assumed the palette's selected
row; the final test uses Alt and selects the exact named palette action.

Reproduction: put Node 22 on PATH; prefix each command with
`python3 scripts/release-check.py --cwd apps/desktop --env CI=1 --`.
For the browser cases run `node node_modules/@playwright/test/cli.js test
--project=renderer --workers=1` followed by the selection above. The first-use
fixture's host data is synthetic, so dashboard usage values are unavailable.
