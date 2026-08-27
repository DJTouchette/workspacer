/**
 * Staged screenshot rig, FLEET-MANAGER arc — the shots the manager work needs
 * that `scripts/shoot.mjs` predates. One run captures:
 *   sidebar-nesting   — a manager's crew indented under it in the sidebar
 *   overview          — the Overview with the Fleet Manager hero, column folded
 *   spawn-dialog      — the dispatch dialog
 *   fleet-manager     — the manager pane: an ask, a dispatch, a [fleet] wake
 *   structured-result — that same wake's validated result, as a card
 *   brief-board       — every brief as swimlanes, mid-drag over Archive
 *   fleet-deck        — the deck, restaged for the Dispatch/In flight rename
 *   agent-gui         — a worker's own pane: work log, diffs, changed files
 *   review-pane       — git status + unified diff over the staged repo
 *   editor            — the sandboxed editor plugin on a real source file
 *   ask-question      — the same worker with a pending AskUserQuestion
 *   triage-inbox      — the inbox drawer over the deck
 * The /m phone shots have their own rig: scripts/shootMobile.mjs.
 *
 *   npm run shoot:fleet
 *
 * SAME TWO ISOLATIONS AS shoot.mjs, AND FOR THE SAME REASONS. Read that file's
 * header — it is the authority. In short:
 *
 *  1. STATE — HOME plus all four XDG dirs, because XDG_CONFIG_HOME is set in a
 *     desktop session and Electron's getPath('userData') prefers it. The run
 *     hard-aborts if userData lands outside the stage.
 *  2. PORTS — daemonUtils.ts PORTS are literal constants and startup SIGTERMs
 *     whatever holds them, so a second instance TAKES the live app's daemons
 *     rather than getting its own. The rootless network namespace
 *     (`unshare -Urn`) gives the stage a private 127.0.0.1 whose `lsof` sweep
 *     cannot see the host. THE GUARD BELOW IS LOAD-BEARING: do not remove it,
 *     and never run the inner node command directly.
 *
 * WHAT IS NEW HERE. Two pieces of stage state that hooks cannot carry:
 *
 *  - A pre-written SAVED SESSION (`<configDir>/sessions/staged.yaml`). The
 *    manager arc is about RELATIONSHIPS between sessions — a named manager with
 *    a crew nested under it — and `name`, `kind: supervisor` and `parentId`
 *    live on the AgentWorkspace, not on any hook and not on any claudemon
 *    snapshot. So the stage restores them the way a real relaunch does, through
 *    the app's own boot restore (useSessionLifecycle reads the most recent file
 *    in that directory). Two consequences drive the ORDER of this script:
 *      · the fleet must be seeded BEFORE the renderer's post-restore reconcile
 *        runs, or every restored card is marked stopped and respawned (which a
 *        credential-less stage cannot do). So seeding happens right after the
 *        daemon's hook port opens, in parallel with the window coming up.
 *      · restored cards already own their session ids, so auto-adoption
 *        correctly leaves them alone.
 *    (Playwright's main-process bridge is NOT a route to `setSpawnMeta`: the
 *    bridged function is compiled outside any module, so `require`,
 *    `process.mainModule` and dynamic `import()` are all unavailable there.
 *    Verified, not assumed.)
 *  - a pre-written `config.yaml` in the stage, because the Board's swimlanes
 *    come from `config.projects` (briefBoardService.boardLaneTargets), not
 *    from the live fleet.
 *
 * Captures at 1600x862 into $SHOT_OUT (default /tmp/wks-shoot-fleet/out). The
 * webp conversion into landing/shots/ happens outside this script.
 */
import { _electron as electron } from 'playwright';
import * as fs from 'fs';
import * as path from 'path';
import {
  fireQuestion,
  seedBriefs,
  seedFleet,
  seedProjectTree,
  seedSavedSession,
  waitForHook,
} from './shootFleetFixture.mjs';

const APP = path.resolve(import.meta.dirname, '..');
// The stage's HOME is also the manager's root and the Board's fleet lane, and
// it SHOWS: the Overview lists each project's path. Keep it short and neutral
// — a capture is a public artefact and must not carry the rig's own name, let
// alone anything from the real profile.
const STAGE = process.env.STAGE_HOME || '/tmp/dev';
const OUT = process.env.SHOT_OUT || '/tmp/wks-shoot-fleet/out';

if (!process.env.WKS_SHOOT_NETNS) {
  console.error(
    'Refusing to run outside a network namespace — the app would SIGTERM the\n' +
      "live instance's daemons (see the header comment).\n\n" +
      'Run it as:\n' +
      '  unshare -Urn --map-root-user bash -c \\\n' +
      "    'ip link set lo up; WKS_SHOOT_NETNS=1 node scripts/shootFleet.mjs'",
  );
  process.exit(1);
}

fs.rmSync(STAGE, { recursive: true, force: true });
fs.mkdirSync(`${STAGE}/.config/workspacer`, { recursive: true });
fs.mkdirSync(OUT, { recursive: true });

// Briefs first: the Board reads them off disk, and config has to name their
// directories before the app boots.
const projectDirs = seedBriefs(STAGE);

// A stage config. `projects` is what gives the Board its swimlanes; the rest
// just saves a click (GUI chat rather than the terminal default) or keeps the
// capture stable (no animations).
const yaml = [
  'ui:',
  '  theme: everforest',
  '  animations: false',
  '  sidebarWidth: 296',
  'claude:',
  '  defaultView: gui',
  '  transport: pty',
  'agents:',
  // Without this the dispatch dialog opens on the app's OWN cwd — the checkout
  // the rig is running from — and that path is legible in the capture.
  `  defaultCwd: ${JSON.stringify(path.join(STAGE, 'atlas'))}`,
  'projects:',
  ...Object.entries(projectDirs).flatMap(([name, dir]) => [
    `  ${JSON.stringify(dir)}:`,
    `    label: ${name}`,
    `    delivery: ${name === 'orbital' ? 'pr' : 'local'}`,
  ]),
  '',
].join('\n');
fs.writeFileSync(`${STAGE}/.config/workspacer/config.yaml`, yaml);

// A real git repository with real source under the ledger project. The Review
// pane and the editor plugin both read the filesystem, so neither can be staged
// from a hook the way the chat surfaces are.
seedProjectTree(STAGE);

// The saved layout the app restores at boot: it is what carries the manager's
// name, its supervisor kind, and each worker's parentId.
seedSavedSession(STAGE);

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

// Prove state isolation before anything is captured or written.
const userData = await app.evaluate(async ({ app }) => app.getPath('userData'));
if (!userData.startsWith(STAGE)) {
  console.error(`ABORT: userData=${userData} is outside ${STAGE} — would touch the real profile`);
  await app.close();
  process.exit(2);
}
console.log(`isolated userData = ${userData}`);

await waitForHook();
const ids = await seedFleet(STAGE);
console.log(`seeded ${ids.length} sessions`);

const page = await app.firstWindow();
await page.waitForSelector('.app-root', { timeout: 40000 });
await page.setViewportSize({ width: 1600, height: 862 });
await page.waitForTimeout(6000);

const shot = async (name) => {
  await page.screenshot({ path: `${OUT}/${name}.png` });
  console.log(`captured ${OUT}/${name}.png`);
};

/**
 * Best-effort staging step. A scroll target that never appears is a reason to
 * look at the frame, not a reason to lose the seven frames already taken and
 * the four still to come — so it is logged loudly and the run continues. Every
 * capture is inspected by hand before it ships either way.
 */
const soft = async (label, fn) => {
  try {
    await fn();
  } catch (err) {
    console.error(`STEP FAILED (${label}): ${err.message.split('\n')[0]}`);
  }
};

// ── sidebar-nesting: the crew folded under the manager, Overview behind ──
// The sidebar is the subject; the Overview keeps the rest of the frame honest
// (it is what is actually behind the column at boot).
await page.waitForSelector('text=Fleet Manager', { timeout: 20000 });
await shot('sidebar-nesting');

// ── overview: the same space with the column folded away ──
// Re-captured rather than left alone: the shipped one predates the Fleet
// Manager hero its own caption on the page describes, still says "New agent…"
// where the app now says "Dispatch agent", and labels its tiles WORKING /
// NEED YOU where they now read In flight / Waiting. Collapsed so it is not the
// sidebar shot over again.
await page.locator('[title^="Collapse sidebar"]').first().click();
await page.waitForTimeout(1200);
await shot('overview');

// ── spawn-dialog: the dispatch dialog ──
// Also re-captured, and this one is not only stale: the shipped capture has the
// REAL working directory of the machine it was taken on legible in its path
// field, on a public page.
await page.keyboard.press('Control+Shift+KeyN');
await page
  .getByText('Give it a home directory', { exact: false })
  .first()
  .waitFor({ timeout: 15000 });
await page.waitForTimeout(1500);
await shot('spawn-dialog');
await page.keyboard.press('Escape');
await page.waitForTimeout(800);
await page.locator('[title^="Expand sidebar"]').first().click();
await page.waitForTimeout(1000);

// ── fleet-manager: the manager pane mid-conversation ──
await page.locator('[role="button"][title^="Fleet Manager —"]').first().click();
await page.waitForTimeout(1200);
// A fresh profile's transport is PTY and the pane mounts at adoption time, so
// flip to GUI through the status-bar toggle rather than pre-seeding storage.
const gui = page.getByRole('button', { name: 'GUI', exact: true });
if (await gui.count()) await gui.first().click();
await page.getByText('Two dispatches out', { exact: false }).first().waitFor({ timeout: 20000 });
await page.waitForTimeout(2500);
// The pane opens pinned to the tail. Scroll the user's ask up to the top so the
// frame is the ARC — ask, dispatch summary, then the wake coming back — rather
// than the tail the next shot is about.
await page.getByText('the fee totals on ledger', { exact: false }).first().scrollIntoViewIfNeeded();
await page.waitForTimeout(1200);
await shot('fleet-manager');

// ── structured-result: the same pane, at the tail, on the result card ──
// Not a second staged card: this IS the one the wake above carried, in the
// conversation that produced it.
await page.getByText('ROOT CAUSE', { exact: false }).first().scrollIntoViewIfNeeded();
await page.waitForTimeout(1200);
await shot('structured-result');

// ── brief-board: every project's brief as swimlanes, mid-drag over Archive ──
await page.keyboard.press('Control+k');
await page.waitForTimeout(500);
await page.keyboard.type('Brief Board');
await page.waitForTimeout(600);
await page.keyboard.press('Enter');
await page.waitForSelector('text=Direction', { timeout: 20000 });
await page.waitForTimeout(2000);

/**
 * Put the board into a real mid-drag state: a `dragstart` on a card in the
 * fleet lane, then — a tick later — `dragover` on that lane's Archive column.
 * Native HTML5 drag cannot be driven by mouse events in a headless capture, but
 * the pane's own handlers are what render the state, so this dispatches the
 * events the browser would, with a real DataTransfer.
 *
 * THE TWO STEPS MUST NOT SHARE A TICK. `dragstart` sets React state
 * (`dragging`), and a column only accepts a `dragover` once it has re-rendered
 * with that state — firing both synchronously left every column inert and the
 * capture looked identical to one taken at rest.
 */
const dragged = await page.evaluate(() => {
  // The FIRST lane — the manager's own fleet brief, and the one at the top of
  // the frame. A drag in a lane below the fold is a drag nobody can see.
  const lane = document.querySelector('[data-lane]');
  if (!lane) return null;
  const laneKey = lane.getAttribute('data-lane');
  const cols = [...document.querySelectorAll(`[data-lane="${laneKey}"]`)];
  const recently = cols.find((c) => c.getAttribute('data-column') === 'Recently');
  const card = recently?.querySelector('[draggable="true"]');
  if (!card) return null;
  const dt = new DataTransfer();
  window.__wksDrag = { dt, laneKey };
  card.dispatchEvent(new DragEvent('dragstart', { bubbles: true, dataTransfer: dt }));
  return { laneKey, card: card.textContent?.slice(0, 60) ?? '' };
});
await page.waitForTimeout(500);
const hovering = await page.evaluate(() => {
  const held = window.__wksDrag;
  if (!held) return false;
  const archive = [...document.querySelectorAll(`[data-lane="${held.laneKey}"]`)].find(
    (c) => c.getAttribute('data-column') === 'archive',
  );
  if (!archive) return false;
  const r = archive.getBoundingClientRect();
  archive.dispatchEvent(
    new DragEvent('dragover', {
      bubbles: true,
      cancelable: true,
      dataTransfer: held.dt,
      clientX: r.left + r.width / 2,
      clientY: r.top + 40,
    }),
  );
  // The pane paints the hovered column with the accent border; report what it
  // actually looks like rather than assuming the event took.
  return getComputedStyle(archive).borderStyle !== 'none';
});
console.log(
  `board drag: ${dragged ? JSON.stringify(dragged) : 'FAILED'} · archive hover: ${hovering}`,
);
await page.waitForTimeout(600);
await shot('brief-board');

// ── fleet-deck: back to the Overview, then up to fleet altitude ──
await page.evaluate(() => {
  const el = [...document.querySelectorAll('[draggable="true"]')][0];
  el?.dispatchEvent(new DragEvent('dragend', { bubbles: true }));
});
await page.locator('[title^="Overview —"]').first().click();
await page.waitForTimeout(1000);
await page.getByTitle('Open the Fleet').first().click();
await page.waitForTimeout(2500);
await shot('fleet-deck');

// ── agent-gui: one worker's own pane, not the fleet's view of it ───────────
// The ledger worker, because it is the only one that has finished a piece of
// work: a read, three edits with inline diffs, a test run, an end-of-turn
// changed-files card, and three files in the inspector rail. The same three
// files the Review and editor shots below are of.
//
// ORDER MATTERS FROM HERE DOWN, and not for tidiness. Piloting an agent
// auto-dismisses that agent's inbox items (AttentionContext: you are already
// looking at it), so the question has to be fired from the FLEET view or the
// inbox capture is a drawer with one item in it. And ask-question is taken
// before the Review/editor tabs exist so that frame is a chat pane, not a chat
// pane with two unrelated tabs beside it.

// The deck is an overlay and it covers the sidebar the card lives in.
await page.keyboard.press('Escape');
await page.waitForTimeout(1200);
const ledgerCard = page.locator('[role="button"][title*="reconcile the fee rounding"]');
if (!(await ledgerCard.count())) {
  console.error(
    'ABORT: no ledger card in the sidebar. Titles present:',
    JSON.stringify(
      await page
        .locator('[role="button"][title]')
        .evaluateAll((els) => els.map((e) => e.getAttribute('title')).slice(0, 20)),
    ),
  );
  await app.close();
  process.exit(3);
}
await ledgerCard.first().click();
await page.waitForTimeout(1500);
const workerGui = page.getByRole('button', { name: 'GUI', exact: true });
if (await workerGui.count()) await workerGui.first().click();
await page.getByText('CHANGED FILES', { exact: false }).first().waitFor({ timeout: 20000 });
// NOT the inspector rail (mod+shift+e). It opens, but it renders "No files
// changed yet" against a session whose edits arrived as hooks rather than
// through a live turn, so it costs 340px of frame and says nothing true.
// Pin the frame to the END of the turn — the last edits and the card that rolls
// them up — rather than wherever the restore left the scroll.
await soft('agent-gui scroll to tail', async () => {
  await page.getByText('Scroll to bottom', { exact: false }).first().click({ timeout: 6000 });
});
await page.waitForTimeout(2000);
await shot('agent-gui');

// ── triage-inbox: the drawer over the deck, from the fleet ─────────────────
// The question is fired HERE, at fleet altitude, so it reaches the inbox at all.
await page.locator('[title^="Overview —"]').first().click();
await page.waitForTimeout(1200);
await page.getByTitle('Open the Fleet').first().click();
await page.waitForTimeout(2000);
await fireQuestion();
await page.waitForTimeout(3000);
await soft('inbox open', async () => {
  await page.keyboard.press('Control+k');
  await page.waitForTimeout(500);
  await page.keyboard.type('Toggle Inbox');
  await page.waitForTimeout(600);
  await page.keyboard.press('Enter');
});
await page.waitForTimeout(2500);
await shot('triage-inbox');

// ── ask-question: the same question, in the agent's own pane ───────────────
await page.keyboard.press('Escape');
await page.waitForTimeout(700);
await page.keyboard.press('Escape');
await page.waitForTimeout(1200);
await ledgerCard.first().click();
await page.waitForTimeout(2500);
await soft('ask-question scroll to tail', async () => {
  await page.getByText('Scroll to bottom', { exact: false }).first().click({ timeout: 6000 });
});
await page.waitForTimeout(2000);
await shot('ask-question');

// ── review-pane: the same three files, as git sees them ────────────────────
await soft('review pane open', async () => {
  await page.keyboard.press('Control+k');
  await page.waitForTimeout(500);
  await page.keyboard.type('Review Changes');
  await page.waitForTimeout(600);
  await page.keyboard.press('Enter');
  await page.waitForTimeout(4000);
});
// DO NOT click a file in the Review tree to change the selected diff: the tree's
// click handler is "open in editor", so it opens an editor tab over the pane and
// the capture becomes a second editor shot. The default selection is the frame.
await page.waitForTimeout(2500);
await shot('review-pane');

// ── editor: the sandboxed editor plugin, opened ON a file ──────────────────
// Through the app's own open-in-editor bus rather than the palette: the palette
// entry opens the plugin with no file, which is what the shipped capture shows
// and why it says "No file open" under a caption promising syntax highlighting.
await page.evaluate(() => {
  window.dispatchEvent(
    new CustomEvent('editor:open-file', {
      detail: { path: '/tmp/dev/ledger/internal/fees/round.go', cwd: '/tmp/dev/ledger' },
    }),
  );
});
await page.waitForTimeout(6000);
await shot('editor');

await app.close();
