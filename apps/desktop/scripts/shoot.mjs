/**
 * Staged screenshot rig — runs the REAL app against a throwaway profile and
 * captures it offscreen, safely, while your own workspacer keeps running.
 *
 *   npm run shoot
 *
 * ## Why the wrapper exists
 *
 * Two independent things have to be isolated, and the older rig only did one.
 *
 * 1. STATE — `HOME` plus all four XDG dirs. Isolating `HOME` alone is NOT
 *    enough: `XDG_CONFIG_HOME` is usually set explicitly in a desktop session,
 *    and Electron's `getPath('userData')` prefers it, so a HOME-only run opens
 *    the real `~/.config/workspacer/workspacer.db`. The script hard-aborts if
 *    `userData` lands outside the stage.
 *
 * 2. PORTS — and this is the one that bites. `lib/daemonUtils.ts` PORTS are
 *    literal constants (7890/7891/7895/7897) with no env override, and startup
 *    runs `lsof -ti :<port>` then SIGTERMs whatever holds each one. A second
 *    instance therefore does not get its own ports, it TAKES them: launching
 *    the e2e suite or a staging rig beside a live app kills that app's daemons
 *    and every agent with them. This happened on 2026-07-19.
 *
 *    The fix is a rootless network namespace. Inside `unshare -Urn` the staged
 *    app gets a private 127.0.0.1, binds the same port numbers without
 *    conflict, and its `lsof` sweep cannot see the host's processes at all.
 *    No root, no config change, no ports plumbed through the app.
 *
 * `--ozone-platform=headless` keeps the window off the real compositor, so a
 * run never flashes a window or steals focus mid-session. (There is no Xvfb on
 * this machine and none is needed.)
 *
 * ## Status
 *
 * Full run: boots, seeds a fabricated 4-agent fleet (see shootFixture.mjs for
 * how sessions are observed into existence without credentials), then captures
 *   - boot.png        — Overview + populated sidebar
 *   - agent-gui.png   — the hero agent's GUI pane: work log + inspector rail
 *   - ask-question.png — the same pane with a pending 4-option AskUserQuestion
 * All at 1600x862. The webp conversion into landing/shots/ happens outside
 * this script (see the handoff doc).
 */
import { _electron as electron } from 'playwright';
import * as fs from 'fs';
import * as path from 'path';
import { seedFleet, fireQuestion, waitForHook } from './shootFixture.mjs';

const APP = path.resolve(import.meta.dirname, '..');
const STAGE = process.env.STAGE_HOME || '/tmp/wks-shoot/home';
const OUT = process.env.SHOT_OUT || '/tmp/wks-shoot/out';

if (!process.env.WKS_SHOOT_NETNS) {
  console.error(
    'Refusing to run outside a network namespace — the app would SIGTERM the\n' +
      "live instance's daemons (see the header comment).\n\n" +
      'Run it as:\n' +
      '  unshare -Urn --map-root-user bash -c \\\n' +
      "    'ip link set lo up; WKS_SHOOT_NETNS=1 node scripts/shoot.mjs'",
  );
  process.exit(1);
}

fs.rmSync(STAGE, { recursive: true, force: true });
fs.mkdirSync(`${STAGE}/.config`, { recursive: true });
fs.mkdirSync(OUT, { recursive: true });

const app = await electron.launch({
  cwd: APP,
  args: ['.', '--no-sandbox', '--ozone-platform=headless', '--disable-gpu'],
  env: {
    ...process.env,
    ELECTRON_DEV: '',
    HOME: STAGE,
    XDG_CONFIG_HOME: `${STAGE}/.config`,
    XDG_DATA_HOME: `${STAGE}/.local/share`,
    XDG_STATE_HOME: `${STAGE}/.local/state`,
    XDG_CACHE_HOME: `${STAGE}/.cache`,
  },
  timeout: 60000,
});

const page = await app.firstWindow();
await page.waitForSelector('.app-root', { timeout: 40000 });
await page.setViewportSize({ width: 1600, height: 862 });

// Prove state isolation before capturing anything. If this resolves to the
// real profile the run must not continue.
const userData = await app.evaluate(async ({ app }) => app.getPath('userData'));
if (!userData.startsWith(STAGE)) {
  console.error(`ABORT: userData=${userData} is outside ${STAGE} — would touch the real profile`);
  await app.close();
  process.exit(2);
}
console.log(`isolated userData = ${userData}`);

// Seed the fleet by observing sessions into existence (transcript + hooks),
// since a credential-less stage cannot spawn real agents.
await waitForHook();
const ids = await seedFleet(STAGE);
console.log(`seeded ${ids.length} sessions`);
await page.waitForTimeout(4000);

await page.screenshot({ path: `${OUT}/boot.png` });
console.log(`captured ${OUT}/boot.png`);

// ── agent-gui: the hero pane in GUI mode with the inspector rail open ──
// Sidebar agent cards are div[role=button] titled "<name> — <state>…".
await page.locator('[role="button"][title^="workspacer —"]').first().click();
// A fresh profile's claude.defaultView is 'terminal' (config_defaults.json),
// and the pane mounts at adoption time — so flip to GUI and open the rail
// via their status-bar toggles rather than pre-seeding localStorage.
await page.getByRole('button', { name: 'GUI', exact: true }).click();
await page.locator('[title^="Show inspector"]').first().click();
// The GUI conversation is fed from the transcript; wait for the final
// assistant line so the whole work log is in.
await page.getByText('ready to flip public', { exact: false }).first().waitFor({ timeout: 15000 });
await page.waitForTimeout(2500); // inspector meters + changed-files card settle
await page.screenshot({ path: `${OUT}/agent-gui.png` });
console.log(`captured ${OUT}/agent-gui.png`);

// ── ask-question: same pane, now blocked on a 4-option question ──
// Close the rail first so the picker + changed-files cards get the width.
await page.locator('[title="Hide inspector"]').first().click();
await fireQuestion();
await page.getByText('Claude is asking you', { exact: false }).first().waitFor({ timeout: 15000 });
await page.waitForTimeout(1500);
await page.screenshot({ path: `${OUT}/ask-question.png` });
console.log(`captured ${OUT}/ask-question.png`);

await app.close();
