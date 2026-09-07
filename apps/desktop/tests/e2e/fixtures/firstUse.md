# First-use browser proof

Run `npm run test:e2e:first-use` from `apps/desktop` (Node 22 and Playwright Chromium).
The renderer project also includes this fixture in the existing CI project selection.

`firstUseHarness.tsx` mounts the production App, ConfigProvider and PluginsProvider.
It clones shipped `configDefaults.generated.ts`, sets `onboardingDismissed: false`,
and starts with no sessions or shared layout. The browser gets a fresh storage context
per test. Only theme, provider discovery and fake launch outcomes vary; safe permission
and provider/model defaults remain shipped values. PTY cases explicitly select terminal
transport through the production dialog because current Claude defaults are stream.

The fixture replaces electronAPI before importing App. Provider discovery, runtime,
config/layout persistence, session updates and launches stay in memory. Tests block
requests outside the ephemeral fixture origin. No backend installation, live daemon,
provider process, profile reset or credentials are involved.

Tests cover welcome, direct shortcut, palette prompt, Guide welcome/pane and Fleet
Manager launches; rejected/malformed spawns; retained input and retry; cancellation;
missing/unknown/old-host detection; overrides/recheck; Codex-only first tasks; keyboard
focus and safe permissions at 360/1280 pixels in Light/Dracula. Screenshots are written
to Playwright's ignored `test-results` directory. `fleetContextMenu.test.ts` additionally
pins retained chat/Back/menu/send/results behavior.

The browser captures the production renderer's spawn payload. The companion
`claudeSpawn.test.ts` and `managedSpawn.test.ts` exercise production backend spawn
helpers through mocked daemon launch boundaries, including rejected launch then retry
with one firstMessage per launch and no permission bypass. These are deterministic
boundary checks, not packaged Electron or real provider/authentication verification.


P1/P2 coverage adds read-only host lifecycle (slow startup, claudemon down,
hub-only degradation, ready/unknown hosts), exact cwd/owner checks, stale git
reply rejection, remote paths with no local probe, and a dismissible chat note.
Main daemon-adoption tests exercise the real startup owner using mocked health
and child-process boundaries. Facade readiness checks its existing `hubConnected`
health field, not just HTTP liveness. Web/remote/old hosts explicitly remain
unknown until they provide an owner-side contract; they do not borrow IPC facts.

Production App tests step through working, a question, and result/prose/malformed
replies. A separate first managed chat renders the shared fleet wake format for
validated escalation/results and missing/malformed result diagnoses. Fixtures
supply those boundary messages, not a fake UI completion rule. The restart fixture
stores only its fake host config/layout/conversation in sessionStorage and reloads
the actual App. Boot reconciliation resumes the stopped id exactly once and keeps
the conversation and guidance dismissal. It makes no manager graph persistence
claim. Existing Fleet C tests continue to cover Back, selection, drafts, and live
iframe retention.
