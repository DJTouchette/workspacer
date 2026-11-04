#!/usr/bin/env node
/**
 * Full-app design screenshot rig. See README.md in this folder.
 *
 *   node scripts/design-shots/shoot-all.mjs [--out DIR] [--setting ui.designV2=true]...
 *
 * Boots the REAL hub (contract fixture build) on an ephemeral port with scratch
 * state, serves the REAL web bundle at /app/, fakes only the far side of the
 * capability boundary with the invented world in world.mjs, and walks headless
 * Chromium through every major screen.
 */
import * as fs from 'fs';
import * as path from 'path';
import { createRequire } from 'module';
import { ensureHubBinary, ensureWebBundle, startStage, DESKTOP_DIR } from './stage.mjs';
import { makeWorld } from './world.mjs';
import { SHOTS } from './shots.mjs';

// ── args ────────────────────────────────────────────────────────────────────
const argv = process.argv.slice(2);
const opt = {
  out: process.env.SHOT_OUT || path.join(DESKTOP_DIR, 'design-shots-out'),
  state: process.env.SHOT_STATE || null,
  settings: [],
  locals: [],
  only: null,
  skipBuild: process.env.SHOT_SKIP_BUILD === '1',
  forceBuild: false,
  serve: false,
  scale: 2,
  list: false,
};
// Env form of --setting, for CI/one-liners: SHOT_SETTINGS="ui.designV2=true,ui.foo=1"
for (const kv of (process.env.SHOT_SETTINGS || '').split(',').filter(Boolean)) opt.settings.push(kv);
for (let i = 0; i < argv.length; i++) {
  const a = argv[i];
  const next = () => argv[++i];
  if (a === '--out') opt.out = next();
  else if (a === '--state') opt.state = next();
  else if (a === '--setting') opt.settings.push(next());
  else if (a.startsWith('--setting=')) opt.settings.push(a.slice(10));
  else if (a === '--local') opt.locals.push(next());
  else if (a === '--only') opt.only = next();
  else if (a === '--skip-build') opt.skipBuild = true;
  else if (a === '--rebuild') opt.forceBuild = true;
  else if (a === '--serve') opt.serve = true;
  else if (a === '--scale') opt.scale = Number(next());
  else if (a === '--list') opt.list = true;
  else if (a === '-h' || a === '--help') {
    console.log(fs.readFileSync(path.join(import.meta.dirname, 'README.md'), 'utf8'));
    process.exit(0);
  } else {
    console.error(`unknown argument: ${a}`);
    process.exit(2);
  }
}
if (opt.list) {
  for (const s of SHOTS) console.log(`${s.id}  ${s.desc}`);
  process.exit(0);
}
opt.state ||= path.join(opt.out, '.rig-state');

/** "ui.designV2=true" → { ui: { designV2: true } }. Values parse as JSON when
 *  they can (true/false/numbers/quoted strings/objects), else stay strings. */
function parseSetting(kv) {
  const eq = kv.indexOf('=');
  if (eq < 1) throw new Error(`--setting expects key.path=value, got ${kv}`);
  const keyPath = kv.slice(0, eq).split('.');
  const raw = kv.slice(eq + 1);
  let value;
  try {
    value = JSON.parse(raw);
  } catch {
    value = raw;
  }
  const root = {};
  let cur = root;
  keyPath.forEach((k, i) => {
    if (i === keyPath.length - 1) cur[k] = value;
    else cur = cur[k] = {};
  });
  return root;
}
const { deepMerge } = await import('./stage.mjs');
const configOverrides = opt.settings.map(parseSetting).reduce((a, b) => deepMerge(a, b), {});
const localOverrides = Object.fromEntries(
  opt.locals.map((kv) => {
    const eq = kv.indexOf('=');
    return [kv.slice(0, eq), kv.slice(eq + 1)];
  }),
);

// ── playwright: the repo's copy if its browser is installed, else the global ──
async function loadChromium() {
  const candidates = [
    () => createRequire(path.join(DESKTOP_DIR, 'package.json'))('playwright'),
    // A globally installed playwright (NODE_PATH / global root) whose browsers
    // are present even when the repo's pinned version's are not.
    ...(process.env.NODE_PATH || '')
      .split(path.delimiter)
      .filter(Boolean)
      .map((p) => () => createRequire(path.join(p, 'noop.js'))('playwright')),
    () => createRequire('/opt/node-tools/node_modules/noop.js')('playwright'),
  ];
  let lastErr;
  for (const load of candidates) {
    try {
      const pw = load();
      const b = await pw.chromium.launch();
      return b;
    } catch (e) {
      lastErr = e;
    }
  }
  throw new Error('could not launch Chromium from any playwright install: ' + lastErr?.message);
}

// ── run ─────────────────────────────────────────────────────────────────────
fs.mkdirSync(opt.out, { recursive: true });
if (!opt.skipBuild || opt.forceBuild) ensureWebBundle({ force: opt.forceBuild });
const hubBin = ensureHubBinary();

const now = Date.now();
const worldFor = (shot) => {
  const w = makeWorld({ now, configOverrides });
  return shot?.world ? shot.world(w, { configOverrides }) : w;
};

let stage = await startStage({ stateRoot: opt.state, world: worldFor(null), hubBin });
const cleanup = async () => {
  try {
    await stage?.stop();
  } catch {
    /* already down */
  }
};
process.on('SIGINT', async () => {
  await cleanup();
  process.exit(130);
});

if (opt.serve) {
  console.log(`\n  /app : ${stage.appUrl}\n  /m   : ${stage.mobileUrl}\n\nCtrl+C to stop.`);
  await new Promise(() => {});
}

const browser = await loadChromium();
const selected = SHOTS.filter((s) => !opt.only || new RegExp(opt.only).test(s.id));
const results = [];
for (const shot of selected) {
  // The contract-fixture hub exits on its own after 10 minutes; recycle well
  // before that so a long run never shoots a dead server.
  if (Date.now() - stage.startedAt > 7 * 60_000) {
    await stage.stop();
    stage = await startStage({ stateRoot: opt.state, world: worldFor(null), hubBin });
  }
  const world = worldFor(shot);
  stage.reset(world);
  await new Promise((r) => setTimeout(r, 150));

  const viewport = shot.viewport ?? { width: 1600, height: 1000 };
  const ctx = await browser.newContext({ viewport, deviceScaleFactor: opt.scale, ...(shot.context ?? {}) });
  const page = await ctx.newPage();
  const errors = [];
  page.on('pageerror', (e) => errors.push(e.message));
  const locals = {
    // A first-run hint strip that sits above every chat; on in the first-run shot only.
    'wks:first-task-guidance-dismissed': 'true',
    ...(shot.localStorage ?? {}),
    ...localOverrides,
  };
  await page.addInitScript((kv) => {
    try {
      for (const [k, v] of Object.entries(kv)) {
        if (v === null) localStorage.removeItem(k);
        else localStorage.setItem(k, v);
      }
    } catch {
      /* storage blocked: shots still render, just with first-run hints */
    }
  }, locals);

  if (shot.initScript) await page.addInitScript(shot.initScript);
  const file = path.join(opt.out, `${shot.id}.png`);
  const t0 = Date.now();
  try {
    await page.goto(shot.url ? shot.url(stage) : stage.appUrl, { waitUntil: 'load' });
    await (shot.ready ?? defaultReady)(page);
    if (shot.run) await shot.run(page, { stage, world });
    await settle(page);
    await page.screenshot({ path: file, fullPage: false });
    results.push({ id: shot.id, ok: true, ms: Date.now() - t0 });
    console.log(`  ✓ ${shot.id}  (${((Date.now() - t0) / 1000).toFixed(1)}s)`);
  } catch (err) {
    try {
      await page.screenshot({ path: file.replace(/\.png$/, '.FAILED.png') });
    } catch {
      /* page gone */
    }
    results.push({ id: shot.id, ok: false, error: String(err?.message ?? err).split('\n')[0] });
    console.log(`  ✗ ${shot.id}: ${String(err?.message ?? err).split('\n')[0]}`);
  }
  if (errors.length) console.log(`    page errors: ${errors.slice(0, 3).join(' | ')}`);
  await ctx.close();
}

await browser.close();
if (stage.unhandled.size)
  console.log(`unmodelled bus methods (answered {ok:true}): ${[...stage.unhandled.keys()].join(', ')}`);
await cleanup();
fs.writeFileSync(path.join(opt.out, 'manifest.json'), JSON.stringify({ at: new Date().toISOString(), settings: opt.settings, results }, null, 2));
const failed = results.filter((r) => !r.ok);
console.log(`\n${results.length - failed.length}/${results.length} shots in ${opt.out}`);
process.exit(failed.length ? 1 : 0);

// ── helpers ─────────────────────────────────────────────────────────────────

/** The app is "up" when the hero agent's card has painted in the sidebar:
 *  the bundle booted, the bus connected, the layout was adopted, snapshots
 *  arrived. `load` alone is long before any of that. */
async function defaultReady(page) {
  await page.getByText('Retire the v1 ingest path').first().waitFor({ timeout: 30_000 });
  await page.waitForTimeout(1200);
}

/** Let transitions and async panels finish, and park the mouse off-canvas so
 *  no hover state leaks into the frame. */
async function settle(page) {
  await page.mouse.move(820, 1);
  await page.waitForTimeout(900);
  await page.evaluate(() => document.fonts?.ready);
}
