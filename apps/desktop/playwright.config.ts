import { defineConfig } from '@playwright/test';

/**
 * Projects exist here for one reason: the specs in `tests/e2e` have WILDLY
 * different costs, and without projects there is no way to ask for the cheap
 * ones.
 *
 * That is not hypothetical. `npm run test:e2e` is `npm run build && npx
 * playwright test` — a full main+renderer+web build followed by every spec — so
 * the only thing anyone could afford on every PR was one file named explicitly
 * in ci.yml. Everything else went unrun, and `mobileClient.test.ts` itself sat
 * red for two days before the day it was wired in.
 *
 * The split is therefore by what a spec NEEDS, not by what it covers:
 *
 *   app       Go toolchain + Chromium. Boots the real hub binary on an
 *             ephemeral port with a scratch state dir, serving the web bundle
 *             it builds itself. No Electron, no claudemon, no xvfb.
 *   mobile    Same shape, for the /m PWA. Already in CI.
 *   renderer  Chromium + its own Vite server on :5199. No Go, no Electron,
 *             no xvfb — cheap, and unwired until now for no reason.
 *   electron  `npm run build` AND a real Electron launch, so a headless runner
 *             also needs xvfb. This is the one that cannot ride every PR.
 *
 * The first three run in CI (`.github/workflows/ci.yml`); `electron` is opt-in
 * with `--project=electron`, which is what `npm run test:e2e` still does after
 * building.
 */
export default defineConfig({
  testDir: './tests/e2e',
  timeout: 30000,
  retries: 0,
  use: {
    trace: 'on-first-retry',
  },
  // Matched by PREFIX, not by an enumeration of filenames: a new `app*` or
  // `mobile*` spec joins the right project by being named, with no second edit
  // to remember. `appSuiteCoverage.test.ts` fails if a spec matches no project
  // at all, because a project split turns "forgot to wire it up" from a loud
  // problem into a silent one — a spec nobody runs looks exactly like a spec
  // that passes.
  projects: [
    { name: 'app', testMatch: /(app[A-Z][A-Za-z]*|stateIsolation)\.test\.ts$/ },
    { name: 'mobile', testMatch: /mobile[A-Z][A-Za-z]*\.test\.ts$/ },
    { name: 'renderer', testMatch: /chatTailPin\.test\.ts$/ },
    { name: 'electron', testMatch: /libraryPane\.test\.ts$/ },
  ],
});
