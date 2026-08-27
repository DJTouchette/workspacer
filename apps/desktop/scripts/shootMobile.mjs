/**
 * Staged screenshot rig for the /m phone PWA.
 *
 *   npm run shoot:mobile
 *
 * WHY THIS ONE NEEDS NO NETWORK NAMESPACE. The desktop rigs (shoot.mjs,
 * shootFleet.mjs) must run inside `unshare -Urn` because the app's daemon ports
 * are literal constants and its startup SIGTERMs whatever holds them. Nothing
 * here starts the app. It starts ONE hub binary, on a port the OS picked as
 * free, with a scratch tokens/layout/push directory and `--brain-scope off` —
 * the same isolation `tests/e2e/fixtures/mobileHub.ts` has used all along. It
 * cannot see, adopt or kill the live instance's daemons.
 *
 * WHY IT IS NOT JUST THAT FIXTURE. Two reasons, both about what ends up in the
 * frame. The e2e fixture's sessions are pointed at the user's REAL repositories
 * (`/home/<user>/Work/...`), which must never reach a marketing shot; and it
 * does not answer `fs.read`, which is the one call the Briefs tab makes. So the
 * fleet and the briefs below are the same invented world as the desktop shots
 * (shootFleetFixture.mjs), and the provider answers `fs.read` out of it.
 *
 * The client under test is the real `/m`, go:embed'd in the hub binary, talking
 * the real wire protocol to the real router. Only the far side of the
 * capability boundary is fabricated.
 */
import { spawn } from 'child_process';
import { chromium } from 'playwright';
import * as fs from 'fs';
import * as net from 'net';
import * as os from 'os';
import * as path from 'path';

const APP = path.resolve(import.meta.dirname, '..');
const REPO = path.resolve(APP, '../..');
const HUB_BIN = path.join(REPO, 'services/hub/hub');
const OUT = process.env.SHOT_OUT || '/tmp/wks-shoot-fleet/out';

// Phone frame — matches the existing mobile-*.webp on the landing page.
const WIDTH = 640;
const HEIGHT = 1098;

const HOST_TOKEN = 'shoot-host-token';
// Briefs are OPERATOR surface: `fs.read` is absent from the view and triage
// tiers, and the client renders a "needs an operator token" empty state rather
// than a brief if the phone holds anything less.
const PHONE_TOKEN = 'shoot-phone-token';

if (!fs.existsSync(HUB_BIN)) {
  console.error(
    `hub binary not found at ${HUB_BIN} (cd services/hub && go build -o hub ./cmd/hub)`,
  );
  process.exit(1);
}
fs.mkdirSync(OUT, { recursive: true });

const freePort = () =>
  new Promise((resolve, reject) => {
    const srv = net.createServer();
    srv.on('error', reject);
    srv.listen(0, '127.0.0.1', () => {
      const port = srv.address().port;
      srv.close(() => resolve(port));
    });
  });

async function waitForHealth(url, timeoutMs = 15000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      if ((await fetch(url + '/health')).ok) return;
    } catch {
      /* not up yet */
    }
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error('hub did not become healthy at ' + url);
}

// ── The staged world ─────────────────────────────────────────────────────────
// Same invented projects as the desktop shots, so the two read as one fleet.

const now = Date.now();
const MANAGER_CWD = '/tmp/dev';

const tool = (id, name, input, status = 'complete', at = now - 600_000) => ({
  id,
  name,
  input,
  status,
  startedAt: at,
  completedAt: status === 'complete' ? at + 900 : undefined,
});

const session = (o) => ({
  ptyId: `pty-${o.sessionId}`,
  status: 'active',
  provider: 'claude',
  transport: 'stream',
  conversation: [],
  activeToolCalls: [],
  completedToolCalls: [],
  fileChanges: [],
  pendingApproval: null,
  pendingQuestions: null,
  subagents: [],
  workflows: [],
  totalToolCalls: 12,
  ...o,
});

const SESSIONS = [
  session({
    sessionId: 'a91c4f20',
    cwd: MANAGER_CWD,
    label: 'Fleet Manager',
    isSupervisor: true,
    ambientState: 'idle',
    lastActivity: now - 170_000,
    statusLine: { modelDisplay: 'Opus 4.8', contextUsedPct: 21, costUSD: 1.84 },
  }),
  session({
    sessionId: '7c41ab90',
    cwd: '/tmp/dev/atlas',
    label: 'atlas: retire the v1 ingest path',
    parentSessionId: 'a91c4f20',
    ambientState: 'streaming',
    lastActivity: now - 4_000,
    statusLine: { modelDisplay: 'Sonnet 5', contextUsedPct: 44, costUSD: 2.9 },
  }),
  session({
    sessionId: '2f9d10e4',
    cwd: '/tmp/dev/ledger',
    label: 'ledger: reconcile the fee rounding',
    parentSessionId: 'a91c4f20',
    ambientState: 'idle',
    lastActivity: now - 260_000,
    // The chat screen renders `snapshot.conversation` directly when it has one
    // (convTurns) — no sessions.conversation round trip needed.
    conversation: [
      {
        role: 'user',
        content:
          'The fee totals are drifting against the gateway. Find out why and fix it. End with a fenced wks-result block.',
        timestamp: now - 900_000,
      },
      {
        role: 'assistant',
        content:
          'The drift is per-line rounding: each line item rounds to a cent, and the sum of twelve roundings is not the rounding of the sum.',
        timestamp: now - 780_000,
      },
      {
        role: 'assistant',
        content: '',
        timestamp: now - 700_000,
        toolCalls: [
          tool('c1', 'Read', { file_path: 'internal/fees/round.go' }),
          tool('c2', 'Grep', { pattern: 'round(li.Fee)' }),
          tool('c3', 'Edit', {
            file_path: '/tmp/dev/ledger/internal/fees/round.go',
            old_string: 'for _, li := range inv.Lines {\n\ttotal += round(li.Fee)\n}',
            new_string:
              'for _, li := range inv.Lines {\n\ttotal += li.Fee\n}\ntotal = round(total)',
          }),
        ],
      },
      {
        role: 'assistant',
        content:
          'Rounding now happens once on the invoice total. Replaying the contract corpus before I call it done.',
        timestamp: now - 520_000,
      },
      {
        role: 'assistant',
        content: '',
        timestamp: now - 400_000,
        toolCalls: [
          tool('c4', 'Bash', { command: 'go test ./internal/fees/...' }),
          tool('c5', 'Bash', { command: 'go run ./cmd/replay --corpus invoices.jsonl' }),
        ],
      },
      {
        role: 'user',
        content: 'does the contract fixture agree, or was it written from the wrong side?',
        timestamp: now - 380_000,
      },
      {
        role: 'assistant',
        content:
          'The fixture was wrong. It was generated from the old per-line total, so it pinned the bug rather than the behaviour. I rewrote its expectations from the gateway’s own figures and re-ran the replay.',
        timestamp: now - 340_000,
      },
      {
        role: 'assistant',
        content: '',
        timestamp: now - 320_000,
        toolCalls: [
          tool('c6', 'Edit', { file_path: 'contracts/fee-rounding-cases.json' }),
          tool('c7', 'Bash', { command: 'go test ./... (services/ledger)' }),
          tool('c8', 'Bash', { command: 'git commit -m "fees: round once on the invoice total"' }),
        ],
      },
      {
        role: 'assistant',
        content:
          'Fixed: rounding happens once on the invoice total instead of per line item. Merged to master as 8b3c17d2, and the replay over 9,412 recorded invoices is clean. The 138 already-issued invoices are untouched — that is a data migration and needs your decision.',
        timestamp: now - 260_000,
      },
    ],
    completedToolCalls: [
      tool('c3', 'Edit', { file_path: 'internal/fees/round.go' }),
      tool('c5', 'Bash', { command: 'go run ./cmd/replay --corpus invoices.jsonl' }),
    ],
    fileChanges: [
      {
        path: '/tmp/dev/ledger/internal/fees/round.go',
        toolName: 'Edit',
        input: { old_string: 'a\nb\nc', new_string: 'a\nb' },
        timestamp: now - 700_000,
      },
    ],
    statusLine: { modelDisplay: 'Opus 4.8', contextUsedPct: 37, costUSD: 4.12 },
  }),
  session({
    sessionId: 'd0b5e733',
    cwd: '/tmp/dev/orbital',
    label: 'orbital: rewrite the getting-started guide',
    parentSessionId: 'a91c4f20',
    ambientState: 'waiting_approval',
    lastActivity: now - 90_000,
    pendingApproval: {
      toolName: 'Bash',
      toolInput: { command: 'npm run docs:publish -- --channel stable' },
      timestamp: now - 90_000,
    },
    statusLine: { modelDisplay: 'Sonnet 5', contextUsedPct: 12, costUSD: 0.38 },
  }),
];

// The briefs `fs.read` serves, keyed by the exact path the client asks for.
const BRIEFS = {
  [`${MANAGER_CWD}/.workspacer/brief.md`]: `# Fleet brief

## Now

- ⚠️ 2026-08-23 **The 138 mismatched ledger invoices need a decision from you** — re-issue or credit. Not dispatched: it is a customer-facing choice, not an engineering one.
- 🚧 2026-08-23 **atlas v1 ingest retirement is in flight** (session:7c41ab90). Scoped to deletion plus the call-site sweep; the schema migration is deliberately out of scope.

## Direction

- Delivery is **local** on atlas and ledger, **PR** on orbital. Workers merge into the checkout and stop there; pushes need an explicit go-ahead.
- Dispatch every lead **verify-then-fix**, never blind-fix. A clean no-op costs one cheap worker; a blind fix costs a conflicting re-implementation.
- ❌ 2026-08-19 **RETRACTION.** "orbital has no test suite" is wrong — it has one, under \`e2e/\`, that CI never ran.

## Recently

- 2026-08-23: **ledger fee rounding reconciled** (8b3c17d2). Rounding moved from per-line-item to the invoice total; the contract fixture was itself wrong.
- 2026-08-21: atlas ingest v2 shipped and has carried all traffic for a week, which is what made the v1 retirement safe to queue.

## User

- Pushes need an explicit go-ahead on every project. Workers merge locally and stop there.
`,
  '/tmp/dev/atlas/.workspacer/brief.md': `# atlas

## Now

- 🚧 2026-08-23 **v1 ingest path retirement is dispatched** (session:7c41ab90). Eleven call sites, nine of them in tests.

## Recently

- 2026-08-21: ingest v2 took over all production traffic; v1 has served nothing for a week.
`,
  '/tmp/dev/ledger/.workspacer/brief.md': `# ledger

## Now

- ⚠️ 2026-08-23 **138 already-issued invoices carry the old, wrong total.** Re-issue or credit is a customer decision and is waiting on the user.

## Recently

- 2026-08-23: **fee rounding reconciled** (8b3c17d2). Contract replay over 9,412 invoices is clean.
`,
  '/tmp/dev/orbital/.workspacer/brief.md': `# orbital

## Now

- ⚠️ 2026-08-22 **Publishing to the stable docs channel needs your approval** — it is the page people land on from search.

## Recently

- 2026-08-20: the e2e suite was wired into CI and immediately caught two dead commands in the docs.
`,
};

// ── Hub + fake provider ──────────────────────────────────────────────────────

const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-m-shoot-'));
const tokensFile = path.join(dir, 'tokens.json');
fs.writeFileSync(
  tokensFile,
  JSON.stringify([
    { token: PHONE_TOKEN, scope: 'operator', label: 'phone', created: new Date().toISOString() },
  ]),
);

const port = await freePort();
const url = `http://127.0.0.1:${port}`;
const hub = spawn(
  HUB_BIN,
  [
    '--addr',
    `127.0.0.1:${port}`,
    '--token',
    HOST_TOKEN,
    '--tokens-file',
    tokensFile,
    '--layout-file',
    path.join(dir, 'layout.json'),
    '--push-dir',
    path.join(dir, 'push'),
    '--brain-scope',
    'off',
  ],
  // stdin stays OPEN: the hub's parentwatch reads a closed stdin as "my parent
  // died" and shuts down at once.
  { stdio: ['pipe', 'pipe', 'pipe'] },
);
hub.stderr?.on('data', (b) => {
  const s = String(b);
  if (/panic|fatal/i.test(s)) console.error('[hub]', s.trim());
});
await waitForHealth(url);
console.log(`staged hub on ${url} (scratch dir ${dir})`);

const ws = new WebSocket(`ws://127.0.0.1:${port}/bus?token=${HOST_TOKEN}`);
await new Promise((resolve, reject) => {
  ws.addEventListener('open', () => resolve());
  ws.addEventListener('error', () => reject(new Error('provider socket failed')));
});
ws.send(
  JSON.stringify({
    op: 'register',
    methods: [
      'sessions.snapshots',
      'sessions.snapshot',
      'sessions.conversation',
      'sessions.recent',
      'fs.read',
      'config.get',
      'library.list',
      'providers.checkAll',
      'providers.listModels',
      'claude.listModels',
    ],
  }),
);
ws.addEventListener('message', (ev) => {
  let f;
  try {
    f = JSON.parse(String(ev.data));
  } catch {
    return;
  }
  if (f.op !== 'call') return;
  const p = f.params ?? {};
  const reply = (result) => ws.send(JSON.stringify({ op: 'result', id: f.id, result }));
  switch (f.method) {
    case 'sessions.snapshots':
      return reply(SESSIONS);
    case 'sessions.snapshot':
      return reply(SESSIONS.find((s) => s.sessionId === p.sessionId) ?? null);
    case 'sessions.conversation':
      return reply({ seq: 1, items: [] });
    case 'sessions.recent':
      return reply([]);
    case 'fs.read': {
      const body = BRIEFS[p.path];
      // A missing brief and a refused read are different answers on this screen
      // and must not be conflated — answer the real ENOENT shape.
      if (body === undefined) return reply({ error: `ENOENT: no such file, open '${p.path}'` });
      return reply({ contents: body });
    }
    case 'config.get':
      return reply({ directories: { favourites: [], recent: [] }, agents: {} });
    default:
      return reply({ ok: true });
  }
});

// ── Capture ──────────────────────────────────────────────────────────────────

const browser = await chromium.launch();
const context = await browser.newContext({
  viewport: { width: WIDTH, height: HEIGHT },
  deviceScaleFactor: 1,
  isMobile: true,
  hasTouch: true,
});
// Headless Chromium denies Notification permission by default, and the client
// honestly says so in a banner across the top of every screen. That banner is
// about the CAPTURE ENVIRONMENT, not about the product — grant it rather than
// ship a warning the user would never see.
await context.grantPermissions(['notifications']);
// grantPermissions alone is not enough: headless Chromium still reports
// `Notification.permission === 'denied'`, and the client honestly renders
// "Notifications are blocked for this site" across the top of every screen.
// That sentence is true of THIS BROWSER, not of the product — on an installed
// PWA with permission granted the same row reads "Get a push when a worker
// needs you". Pin the value the capture is supposed to represent.
await context.addInitScript(() => {
  Object.defineProperty(Notification, 'permission', { get: () => 'granted' });
});
const page = await context.newPage();
await page.goto(`${url}/m?token=${PHONE_TOKEN}`, { waitUntil: 'domcontentloaded' });
await page.waitForSelector('[data-tab="brief"]', { timeout: 20000 });
console.log(`Notification.permission = ${await page.evaluate(() => Notification.permission)}`);
await page.waitForTimeout(2500);

const shot = async (name) => {
  await page.screenshot({ path: `${OUT}/${name}.png` });
  console.log(`captured ${OUT}/${name}.png`);
};

// ── mobile-fleet: the Fleet tab, which is where the client opens ──
// Restaged alongside the Briefs shot rather than left as it was: the shipped
// one is a capture of the user's REAL fleet (their repository paths are legible
// in it) and it predates the tab rename, so it contradicts the page beside it.
await shot('mobile-fleet');

// ── mobile-briefs: the Briefs tab, fleet brief expanded ──
await page.click('[data-tab="brief"]');
await page.waitForTimeout(1500);
// Expand the fleet brief — collapsed, every card is one teaser line, and the
// screen's claim is that the manager's memory is readable from a phone.
await page.locator('[data-brief]').first().click();
await page.waitForTimeout(1500);
console.log(`briefs rendered: ${await page.locator('.brief').count()}`);
await shot('mobile-briefs');

// ── mobile-chat: one dispatch's transcript, opened from its fleet card ──
// LAST, because the chat screen replaces the tab bar with a back chevron —
// there is no tab to return through, so nothing may follow it.
await page.click('[data-tab="fleet"]');
await page.waitForTimeout(1000);
await page.locator('[data-open="2f9d10e4"]').first().click();
await page.waitForTimeout(1800);
// Expand the first collapsed work card: the claim is that a phone shows what
// the agent DID, and a folded "3 steps" row shows only that it did something.
const work = page.locator('[data-work]').first();
if (await work.count()) {
  await work.click();
  await page.waitForTimeout(1200);
}
await shot('mobile-chat');

await browser.close();
try {
  ws.close();
} catch {
  /* already gone */
}
hub.kill('SIGTERM');
fs.rmSync(dir, { recursive: true, force: true });
