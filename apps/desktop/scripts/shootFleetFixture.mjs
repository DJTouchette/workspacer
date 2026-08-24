/**
 * Fabricated FLEET-MANAGER stage for `scripts/shootFleet.mjs`.
 *
 * Sibling of shootFixture.mjs and built on the same trick: a stage profile has
 * no Claude credentials, so sessions are *observed* into existence (transcript
 * JSONL + hooks) rather than spawned. Read that file's header first — the
 * transcript/hook contract is documented there and is unchanged here.
 *
 * WHAT THIS ONE ADDS, and why it needs a second channel.
 *
 * The manager arc is about RELATIONSHIPS between sessions — a manager with a
 * name, and workers nested under it — and no hook carries any of that. A
 * worker's name, its `parentId` and the manager's `kind: 'supervisor'` are
 * AgentWorkspace fields: written by the renderer's spawn path and persisted in
 * the saved layout. So `seedSavedSession` below writes that layout, and the app
 * restores it at boot exactly as it would after a relaunch. Nothing in the app
 * is modified to make this work.
 *
 * The manager's `[fleet]` wake is built by the REAL builder
 * (`dist/main/shared/fleetMessages.js`) rather than hand-written, so the card
 * the GUI renders is the one a live manager would get — if the format ever
 * changes, this fixture follows it instead of silently degrading to a text
 * blob.
 */
import { spawnSync } from 'child_process';
import * as fs from 'fs';
import * as path from 'path';
import { pathToFileURL } from 'url';

const HOOK = 'http://127.0.0.1:7890';
const APP = path.resolve(import.meta.dirname, '..');

const { buildFleetMessage } = await import(
  pathToFileURL(path.join(APP, 'dist/main/shared/fleetMessages.js')).href
);

const encodedCwd = (cwd) => cwd.replace(/[/\\:]/g, '-');

async function post(url, body) {
  const res = await fetch(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  });
  if (!res.ok) throw new Error(`${url} -> ${res.status} ${await res.text()}`);
  return res.json().catch(() => ({}));
}

export async function waitForHook(timeoutMs = 45000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const r = await fetch(`${HOOK}/health`);
      if (r.ok) return true;
    } catch {
      /* not up yet */
    }
    await new Promise((r) => setTimeout(r, 500));
  }
  throw new Error('claudemon hook port never came up');
}

const userLine = (text, ts) =>
  JSON.stringify({ type: 'user', message: { role: 'user', content: text }, timestamp: ts });
const asstLine = (blocks, ts) =>
  JSON.stringify({
    type: 'assistant',
    message: { role: 'assistant', content: blocks },
    timestamp: ts,
  });
const text = (t) => ({ type: 'text', text: t });
const toolUse = (id, name, input) => ({ type: 'tool_use', id, name, input });
const resultLine = (id, content, ts) =>
  JSON.stringify({
    type: 'user',
    message: {
      role: 'user',
      content: [{ type: 'tool_result', tool_use_id: id, content, is_error: false }],
    },
    timestamp: ts,
  });

const iso = (msAgo) => new Date(Date.now() - msAgo).toISOString();
const min = 60000;

// ── The staged world ─────────────────────────────────────────────────────────
// Everything below is invented. The projects do not exist outside the stage,
// and the stage's HOME is a throwaway directory under /tmp.

export const MANAGER = { id: 'a91c4f20', cwd: '/tmp/dev', label: 'Fleet Manager' };

export const WORKERS = [
  {
    id: '7c41ab90',
    label: 'atlas: retire the v1 ingest path',
    cwd: '/tmp/dev/atlas',
    project: 'atlas',
  },
  {
    id: '2f9d10e4',
    label: 'ledger: reconcile the fee rounding',
    cwd: '/tmp/dev/ledger',
    project: 'ledger',
  },
  {
    id: 'd0b5e733',
    label: 'orbital: rewrite the getting-started guide',
    cwd: '/tmp/dev/orbital',
    project: 'orbital',
  },
];

/** The finished worker's validated `wks-result` — the object the manager's
 *  StructuredResultCard renders. Shaped like a real ship-task contract. */
const LEDGER_RESULT = {
  commit: '8b3c17d2',
  merged: true,
  filesChanged: [
    'services/ledger/internal/fees/round.go',
    'services/ledger/internal/fees/round_test.go',
    'contracts/fee-rounding-cases.json',
  ],
  rootCause:
    'Fees were rounded per line item and then summed, so a 12-line invoice drifted by up to 6 cents against the gateway’s single-rounding total. The contract fixture pinned the wrong side.',
  checksRun: [
    'go test ./internal/fees/... — 41/41',
    'go test ./... (services/ledger) — 612/612, matches baseline',
    'contract replay against 9,412 recorded invoices — 0 mismatches (was 138)',
  ],
  followUps: [
    'The gateway reconciliation job still logs the old per-line total; harmless, but it will read as a regression in the dashboard until it is updated.',
  ],
  caveats:
    'Did not backfill the 138 historically mismatched invoices — that is a data migration and needs your call on whether to re-issue or credit.',
};

const LEDGER_REPLY =
  'Fixed: rounding now happens once on the invoice total instead of per line item. ' +
  'Merged to master as 8b3c17d2. The 138 historical mismatches are untouched — that is a ' +
  'data migration and needs your decision.';

/** The wake, through the real builder. */
export const LEDGER_WAKE = buildFleetMessage('worker-finished', [
  {
    label: WORKERS[1].label,
    sessionId: WORKERS[1].id,
    cwd: WORKERS[1].cwd,
    lastReply: LEDGER_REPLY,
    result: JSON.stringify(LEDGER_RESULT, null, 2),
  },
]);

const managerLines = [
  userLine(
    'the fee totals on ledger are off by a few cents against the gateway, and the atlas v1 ingest path is still live even though nothing calls it. take both.',
    iso(14 * min),
  ),
  asstLine(
    [
      text(
        'Two dispatches out, both landing locally per the projects’ delivery policy:\n\n' +
          '- **ledger: reconcile the fee rounding** — `session:2f9d10e4`, opus, worktree `wks/fee-rounding`. ' +
          'Brief says the contract fixture was written from the per-line total, so I told it to verify the fixture before trusting it.\n' +
          '- **atlas: retire the v1 ingest path** — `session:7c41ab90`, sonnet, worktree `wks/retire-v1-ingest`. ' +
          'Scoped to deletion plus the call-site sweep; the schema migration stays out of it.\n\n' +
          'Both asked for a `wks-result`. Nothing else needs me right now, so I’ll stop here and report when they land.',
      ),
    ],
    iso(13 * min),
  ),
  userLine(LEDGER_WAKE, iso(4 * min)),
  asstLine(
    [
      text(
        '`session:2f9d10e4` landed. One rounding pass on the invoice total instead of twelve on the line items, merged as `8b3c17d2`, and the contract replay over 9,412 invoices now comes back clean.\n\n' +
          '**It needs you for one thing:** 138 invoices were already issued with the wrong total. Re-issuing them and crediting the difference are different customer conversations, so I have not dispatched it — say which and I will.',
      ),
      toolUse('tu_brief', 'mcp__workspacer__brief_append', {
        cwd: '/tmp/dev/ledger',
        section: 'Recently',
        line: '2026-08-23: fee rounding reconciled (8b3c17d2). Rounding moved from per-line-item to the invoice total; the contract fixture was itself wrong and was rewritten from the gateway side. 138 already-issued invoices remain mismatched — WAITING ON THE USER to choose re-issue vs credit.',
      }),
    ],
    iso(3 * min),
  ),
  resultLine('tu_brief', 'appended 1 line to /tmp/dev/ledger/.workspacer/brief.md', iso(3 * min)),
  asstLine(
    [
      text(
        'Brief updated on ledger. `session:7c41ab90` is still sweeping call sites — I’ll be woken when it finishes.',
      ),
    ],
    iso(2.8 * min),
  ),
];

const atlasLines = [
  userLine(
    'Retire the v1 ingest path in atlas. Delete it and sweep every call site. Leave the schema migration alone. When you are done, end with a fenced wks-result block.',
    iso(13 * min),
  ),
  asstLine(
    [
      text('Mapping every reference to the v1 ingest package before deleting anything.'),
      toolUse('tu_grep', 'Bash', {
        command: 'rg -l "ingest/v1" --type go',
        description: 'Find every v1 ingest call site',
      }),
    ],
    iso(12 * min),
  ),
  resultLine('tu_grep', '11 files', iso(12 * min)),
  asstLine(
    [
      text('Eleven call sites, nine of them in tests. Rewriting the two real ones onto v2 first.'),
    ],
    iso(6 * min),
  ),
];

// The ledger worker carries the longest log of the staged fleet on purpose: it
// is the one the `agent-gui` and `ask-question` captures are taken of, and a
// GUI shot of a single Edit shows nothing about what the pane is for. The
// three files it touches are the three in LEDGER_RESULT.filesChanged and the
// three the staged git tree is dirtied with (seedProjectTree), so the work log,
// the changed-files card, the inspector rail, the Review pane and the result
// card all agree about what this agent did.
const ledgerLines = [
  userLine(
    'The fee totals are drifting against the gateway. Find out why and fix it. End with a fenced wks-result block.',
    iso(13 * min),
  ),
  asstLine(
    [
      text('Reading the fee path before I change anything.'),
      toolUse('tu_read', 'Read', { file_path: '/tmp/dev/ledger/internal/fees/round.go' }),
    ],
    iso(11 * min),
  ),
  resultLine(
    'tu_read',
    'func InvoiceTotal(inv Invoice) Cents {\n\tvar total Cents\n\tfor _, li := range inv.Lines {\n\t\ttotal += round(li.Fee)\n\t}\n\treturn total\n}',
    iso(11 * min),
  ),
  asstLine(
    [
      text(
        'The drift is per-line rounding: each line item rounds to a cent and the sum of twelve roundings is not the rounding of the sum.',
      ),
      toolUse('tu_fix', 'Edit', {
        file_path: '/tmp/dev/ledger/internal/fees/round.go',
        old_string: 'for _, li := range inv.Lines {\n\t\ttotal += round(li.Fee)\n\t}',
        new_string: 'for _, li := range inv.Lines {\n\t\ttotal += li.Fee\n\t}\n\ttotal = round(total)',
      }),
    ],
    iso(9 * min),
  ),
  resultLine('tu_fix', 'The file internal/fees/round.go has been updated.', iso(9 * min)),
  asstLine(
    [
      text(
        'The contract fixture was written from the per-line total, so it pinned the bug. Rewriting it from the gateway side before I trust a green run.',
      ),
      toolUse('tu_fixture', 'Edit', {
        file_path: '/tmp/dev/ledger/contracts/fee-rounding-cases.json',
        old_string: '"expected_total": 1247',
        new_string: '"expected_total": 1241',
      }),
      toolUse('tu_case', 'Edit', {
        file_path: '/tmp/dev/ledger/internal/fees/round_test.go',
        old_string: '\t}\n}',
        new_string:
          '\t{name: "twelve lines, one rounding", inv: twelveLine(), want: 1241},\n\t}\n}',
      }),
    ],
    iso(7 * min),
  ),
  resultLine('tu_fixture', 'The file contracts/fee-rounding-cases.json has been updated.', iso(7 * min)),
  resultLine('tu_case', 'The file internal/fees/round_test.go has been updated.', iso(7 * min)),
  asstLine(
    [
      text('Replaying the recorded invoices against the corrected fixture.'),
      toolUse('tu_test', 'Bash', {
        command: 'go test ./internal/fees/... && go run ./cmd/replay --corpus invoices.jsonl',
        description: 'Unit tests, then the 9,412-invoice contract replay',
      }),
    ],
    iso(6 * min),
  ),
  resultLine(
    'tu_test',
    'ok  \tledger/internal/fees\t0.212s\t41/41\nreplay: 9412 invoices, 0 mismatches (was 138)',
    iso(6 * min),
  ),
  asstLine([text(LEDGER_REPLY)], iso(5 * min)),
];

const orbitalLines = [
  userLine(
    'Rewrite the getting-started guide against the current CLI. End with a fenced wks-result block.',
    iso(9 * min),
  ),
  asstLine(
    [
      text(
        'Four of the six commands in the guide no longer exist. Rewriting from `--help` output rather than from the old page.',
      ),
    ],
    iso(6 * min),
  ),
];

const soloLines = [
  userLine('why is the nightly image 400MB bigger since tuesday?', iso(6 * min)),
  asstLine(
    [
      text(
        'The builder stage stopped being discarded — a COPY --from was retargeted at it. Confirming before I say so.',
      ),
    ],
    iso(90000),
  ),
];

export const AGENTS = [
  {
    id: MANAGER.id,
    cwd: MANAGER.cwd,
    lines: managerLines,
    settled: true,
    meta: { label: MANAGER.label, isSupervisor: true },
    edits: [
      {
        id: 'tu_brief',
        tool: 'mcp__workspacer__brief_append',
        input: { cwd: '/tmp/dev/ledger' },
      },
    ],
    statusline: {
      model: { display_name: 'Opus 4.8' },
      context_window: { used_percentage: 21, total_input_tokens: 41000, total_output_tokens: 9200 },
      cost: { total_cost_usd: 1.84 },
      rate_limits: {
        five_hour: { used_percentage: 34, resets_at: Math.floor(Date.now() / 1000) + 9400 },
        seven_day: { used_percentage: 58, resets_at: Math.floor(Date.now() / 1000) + 275000 },
      },
    },
  },
  {
    id: WORKERS[0].id,
    cwd: WORKERS[0].cwd,
    lines: atlasLines,
    meta: { label: WORKERS[0].label, parentSessionId: MANAGER.id },
    statusline: {
      model: { display_name: 'Sonnet 5' },
      context_window: { used_percentage: 44, total_input_tokens: 88000, total_output_tokens: 21000 },
      cost: { total_cost_usd: 2.9 },
    },
    // Live tool call → "In flight".
    working: {
      tool_name: 'Bash',
      tool_use_id: 'tu_build',
      tool_input: { command: 'go build ./...', description: 'Rebuild after the sweep' },
    },
  },
  {
    id: WORKERS[1].id,
    cwd: WORKERS[1].cwd,
    lines: ledgerLines,
    settled: true,
    meta: { label: WORKERS[1].label, parentSessionId: MANAGER.id },
    edits: [
      {
        id: 'tu_fix',
        tool: 'Edit',
        input: { file_path: '/tmp/dev/ledger/internal/fees/round.go' },
      },
      {
        id: 'tu_fixture',
        tool: 'Edit',
        input: { file_path: '/tmp/dev/ledger/contracts/fee-rounding-cases.json' },
      },
      {
        id: 'tu_case',
        tool: 'Edit',
        input: { file_path: '/tmp/dev/ledger/internal/fees/round_test.go' },
      },
    ],
    statusline: {
      model: { display_name: 'Opus 4.8' },
      context_window: { used_percentage: 37, total_input_tokens: 71000, total_output_tokens: 18000 },
      cost: { total_cost_usd: 4.12 },
    },
  },
  {
    id: WORKERS[2].id,
    cwd: WORKERS[2].cwd,
    lines: orbitalLines,
    meta: { label: WORKERS[2].label, parentSessionId: MANAGER.id },
    statusline: {
      model: { display_name: 'Sonnet 5' },
      context_window: { used_percentage: 12, total_input_tokens: 22000, total_output_tokens: 3100 },
      cost: { total_cost_usd: 0.38 },
    },
    // Parked approval → "Waiting".
    approval: {
      tool_name: 'Bash',
      tool_input: { command: 'npm run docs:publish -- --channel stable' },
    },
  },
  {
    // Not one of the manager's — proves the sidebar nests a crew without
    // swallowing the agents you started yourself.
    id: 'c58e2b14',
    cwd: '/tmp/dev/pipeline',
    lines: soloLines,
    meta: { label: 'pipeline' },
    statusline: {
      model: { display_name: 'Opus 4.8' },
      context_window: { used_percentage: 6, total_input_tokens: 11000, total_output_tokens: 1400 },
      cost: { total_cost_usd: 0.17 },
    },
  },
];

/** Seed the whole staged fleet: transcripts on disk, then the hooks that bind
 *  each one to a live daemon session. */
export async function seedFleet(stageHome) {
  const projects = path.join(stageHome, '.claude', 'projects');

  for (const a of AGENTS) {
    const dir = path.join(projects, encodedCwd(a.cwd));
    fs.mkdirSync(dir, { recursive: true });
    const transcriptPath = path.join(dir, `${a.id}.jsonl`);
    fs.writeFileSync(transcriptPath, a.lines.join('\n') + '\n');

    await post(`${HOOK}/hook/session_start`, {
      session_id: a.id,
      cwd: a.cwd,
      transcript_path: transcriptPath,
    });
    await post(`${HOOK}/statusline`, { session_id: a.id, ...a.statusline });

    for (const e of a.edits ?? []) {
      const base = { session_id: a.id, cwd: a.cwd, tool_name: e.tool, tool_use_id: e.id };
      await post(`${HOOK}/hook/pre_tool`, { ...base, tool_input: e.input });
      await post(`${HOOK}/hook/post_tool`, {
        ...base,
        tool_input: e.input,
        tool_response: { success: true },
      });
    }
    if (a.settled) await post(`${HOOK}/hook/stop`, { session_id: a.id, cwd: a.cwd });
    if (a.working) await post(`${HOOK}/hook/pre_tool`, { session_id: a.id, cwd: a.cwd, ...a.working });
    if (a.approval)
      await post(`${HOOK}/hook/permission`, { session_id: a.id, cwd: a.cwd, ...a.approval });
  }
  return AGENTS.map((a) => a.id);
}

/**
 * The SAVED LAYOUT the app restores at boot (useSessionLifecycle picks the most
 * recent file in `<configDir>/sessions/`). This is the only channel for the
 * things the manager arc is actually about: the manager's NAME, its
 * `kind: 'supervisor'`, and each worker's `parentId`. None of those live on a
 * hook or on a claudemon snapshot — they are AgentWorkspace fields, written by
 * the spawn path in the renderer and persisted here.
 *
 * `withGlobalWorkspace` injects the Overview card itself, so it is deliberately
 * absent below. Each claude pane carries `attachSessionId` (what binds a
 * restored pane to its daemon session) and `expectHistory` is set by the
 * restore path itself.
 */
export function seedSavedSession(stageHome) {
  const paneFor = (a) => ({
    id: `claude-${a.id}`,
    type: 'claude',
    title: 'Claude',
    cwd: a.cwd,
    provider: 'claude',
    transport: 'pty',
    attachSessionId: a.id,
  });
  const cardFor = (a, extra) => ({
    id: `agent-${a.id}`,
    name: a.meta.label,
    nameSetByUser: true,
    cwd: a.cwd,
    provider: 'claude',
    transport: 'pty',
    model: a.statusline.model.display_name.toLowerCase().split(' ')[0],
    sessionId: a.id,
    tabs: [{ id: `tab-${a.id}`, title: 'Claude', panes: [paneFor(a)], activePaneId: `claude-${a.id}` }],
    activeTabId: `tab-${a.id}`,
    ...extra,
  });

  const agents = AGENTS.map((a) =>
    cardFor(
      a,
      a.meta.isSupervisor
        ? { kind: 'supervisor', supervisor: true, manager: true, toolScope: 'operator' }
        : a.meta.parentSessionId
          ? {
              parentId: `agent-${a.meta.parentSessionId}`,
              dispatchedByManager: true,
              toolScope: 'operator',
            }
          : {},
    ),
  );

  const dir = path.join(stageHome, '.config', 'workspacer', 'sessions');
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(
    path.join(dir, 'staged.yaml'),
    // JSON is valid YAML, and dumping it avoids a js-yaml dependency here.
    JSON.stringify(
      {
        schemaVersion: 1,
        name: 'Default',
        activeAgentId: 'global',
        agents,
        ptyMapping: Object.fromEntries(AGENTS.map((a) => [`claude-${a.id}`, a.id])),
        timestamp: new Date().toISOString(),
      },
      null,
      2,
    ),
  );
}

// ── Briefs, for the Board pane ───────────────────────────────────────────────
// Written into the STAGE, never into a real repo. The shapes mirror this
// codebase's own briefs (dated bullets, disposition glyphs, retractions) so the
// board's status derivation (main/shared/briefBoard) produces real chips
// rather than a wall of unlabelled cards.

const FLEET_BRIEF = `# Fleet brief

## Now

- ⚠️ 2026-08-23 **The 138 mismatched ledger invoices need a decision from you** — re-issue or credit. Not dispatched: it is a customer-facing choice, not an engineering one.
- 🚧 2026-08-23 **atlas v1 ingest retirement is in flight** (session:7c41ab90). Scoped to deletion plus the call-site sweep; the schema migration is deliberately out of scope.

## Direction

- Delivery mode is **local** on atlas and ledger, **PR** on orbital. Workers merge into the checkout and stop there; pushes need an explicit go-ahead.
- Dispatch every lead **verify-then-fix**, never blind-fix. A clean no-op costs one cheap worker; a blind fix costs a conflicting re-implementation.
- ❌ 2026-08-19 **RETRACTION.** "orbital has no test suite" is wrong — it has one, under \`e2e/\`, that CI never ran. Recorded because the claim was acted on twice.

## Recently

- 2026-08-23: **ledger fee rounding reconciled** (8b3c17d2). Rounding moved from per-line-item to the invoice total. The contract fixture was itself wrong and was rewritten from the gateway side.
- 2026-08-22: **orbital getting-started guide dispatched** after the third support ticket about a command that no longer exists.
- 2026-08-21: atlas ingest v2 shipped and has been carrying all traffic for a week, which is what made the v1 retirement safe to queue.

## User

- Pushes need an explicit go-ahead on every project. Workers merge locally and stop there.
- Never dispatch anything that touches customer billing data without asking first.
`;

const FLEET_ARCHIVE = `# Fleet brief — archive

## Recently

- 2026-08-11: ingest v2 dual-write enabled behind a flag; both paths agreed for six days before the cutover.
`;

const PROJECT_BRIEFS = {
  atlas: `# atlas

## Now

- 🚧 2026-08-23 **v1 ingest path retirement is dispatched** (session:7c41ab90). Eleven call sites, nine of them in tests.

## Direction

- The schema migration is a separate piece of work and must not ride along with a deletion sweep.
- Ingest is the only subsystem with a replay corpus. Any change here is verified against it before it is called done.

## Recently

- 2026-08-21: ingest v2 took over all production traffic; v1 has served nothing for a week.
- 2026-08-14: the replay corpus was extended to 40k events after a rounding bug slipped past the old 900-event set.
`,
  ledger: `# ledger

## Now

- ⚠️ 2026-08-23 **138 already-issued invoices carry the old, wrong total.** Re-issue or credit is a customer decision and is waiting on the user.

## Direction

- Fee arithmetic is pinned by \`contracts/fee-rounding-cases.json\`, and the fixture is not authoritative on its own — it was written from the wrong side once already.
- Money paths never get a blind fix. Verify against the gateway's own totals first.

## Recently

- 2026-08-23: **fee rounding reconciled** (8b3c17d2). One rounding pass on the invoice total instead of twelve on the line items; contract replay over 9,412 invoices is clean.
- 2026-08-18: gateway reconciliation job moved to hourly, which is how the drift was noticed at all.
`,
  orbital: `# orbital

## Now

- 🚧 2026-08-23 **getting-started guide rewrite in progress** (session:d0b5e733). Four of the six documented commands no longer exist.
- ⚠️ 2026-08-22 **Publishing to the stable docs channel needs your approval** — it is the page people land on from search.

## Direction

- Delivery here is **PR**, not local merge: docs changes get read by a human before they go out.
- ❌ 2026-08-19 **RETRACTION.** "orbital has no test suite" is wrong. There is one under \`e2e/\`; CI simply never invoked it.

## Recently

- 2026-08-20: the e2e suite was wired into CI and immediately caught two dead commands in the docs.
`,
};

/** Write the stage's briefs and return the project directories, for config. */
export function seedBriefs(stageHome) {
  const write = (dir, name, body) => {
    fs.mkdirSync(path.join(dir, '.workspacer'), { recursive: true });
    fs.writeFileSync(path.join(dir, '.workspacer', name), body);
  };
  write(stageHome, 'brief.md', FLEET_BRIEF);
  write(stageHome, 'brief.archive.md', FLEET_ARCHIVE);

  const dirs = {};
  for (const [name, body] of Object.entries(PROJECT_BRIEFS)) {
    const dir = path.join(stageHome, name);
    write(dir, 'brief.md', body);
    dirs[name] = dir;
  }
  return dirs;
}

// ── A pending question, fired late ───────────────────────────────────────────
/**
 * Park an `AskUserQuestion` on the ledger worker.
 *
 * Deliberately NOT part of `seedFleet`: a pending question flips that agent
 * from settled to waiting and moves the fleet's counts, which every earlier
 * capture in the run is of. Firing it after those are taken means the
 * `ask-question` and `triage-inbox` frames get the state they need without the
 * rest of the run having to be re-thought around it.
 *
 * The hook is `pre_tool`, not `permission`: an AskUserQuestion pre_tool is
 * surfaced as a pending picker on both the daemon and desktop paths and is not
 * gated, so it does not hold the HTTP response open (see shootFixture.mjs).
 */
export async function fireQuestion() {
  await post(`${HOOK}/hook/pre_tool`, {
    session_id: WORKERS[1].id,
    cwd: WORKERS[1].cwd,
    tool_name: 'AskUserQuestion',
    tool_use_id: 'tu_ask',
    tool_input: {
      questions: [
        {
          question: '138 invoices already went out with the old total. How do you want them handled?',
          header: 'Mismatched invoices',
          multiSelect: false,
          options: [
            {
              label: 'Credit the difference (Recommended)',
              description:
                'Issue a credit note for the 1–6 cent delta on each. No re-issue, no new invoice numbers, and the gateway reconciliation lines up on the next run.',
            },
            {
              label: 'Re-issue every invoice',
              description:
                'Void and re-issue all 138 with the corrected total. Cleanest ledger, but every affected customer gets a second invoice email.',
            },
            {
              label: 'Leave them and annotate',
              description:
                'Write the delta into the reconciliation notes and absorb it. Nothing customer-facing changes.',
            },
            {
              label: 'Show me the 138 first',
              description:
                'Dump the affected invoice ids and per-invoice deltas before anything is decided.',
            },
          ],
        },
      ],
    },
  });
}

// ── A real repository under the staged project ───────────────────────────────
/**
 * Give `/tmp/dev/ledger` an actual git tree with actual source in it.
 *
 * Two captures need this and neither can be faked from a hook: the Review pane
 * renders `git status` plus a unified diff, and the editor plugin renders a
 * file tree and a syntax-highlighted buffer. Both read the real filesystem.
 *
 * The tree is committed CLEAN with the bug still in it, then the worker's three
 * edits are applied on top — so the working tree is dirty in exactly the way
 * the worker's log, its changed-files card and its `wks-result` all claim.
 *
 * IDENTITY IS PINNED HERE ON PURPOSE. This runs in the rig's own process, whose
 * HOME is the real one, so an unpinned `git commit` would stamp the user's name
 * and email into a repository that exists to be photographed. `GIT_CONFIG_*`
 * plus the explicit author/committer env keep the real gitconfig out of it.
 */
export function seedProjectTree(stageHome) {
  const root = path.join(stageHome, 'ledger');
  const write = (rel, body) => {
    const abs = path.join(root, rel);
    fs.mkdirSync(path.dirname(abs), { recursive: true });
    fs.writeFileSync(abs, body);
  };

  write('go.mod', 'module ledger\n\ngo 1.23\n');
  write(
    'README.md',
    '# ledger\n\nInvoicing and fee arithmetic.\n\nFee totals are pinned by `contracts/fee-rounding-cases.json`.\nThe fixture is not authoritative on its own — it was written from the\nwrong side once already. Verify against the gateway before trusting a\ngreen run.\n',
  );
  write(
    'internal/fees/round.go',
    `package fees

// Cents is a whole-cent amount. Fees are never carried as floats.
type Cents int64

type Line struct {
	SKU string
	Fee Cents
}

type Invoice struct {
	ID    string
	Lines []Line
}

// InvoiceTotal sums an invoice's line fees.
func InvoiceTotal(inv Invoice) Cents {
	var total Cents
	for _, li := range inv.Lines {
		total += round(li.Fee)
	}
	return total
}

func round(c Cents) Cents {
	if c%1 == 0 {
		return c
	}
	return c + 1
}
`,
  );
  write(
    'internal/fees/round_test.go',
    `package fees

import "testing"

func TestInvoiceTotal(t *testing.T) {
	cases := []struct {
		name string
		inv  Invoice
		want Cents
	}{
		{name: "single line", inv: oneLine(), want: 199},
		{name: "empty invoice", inv: Invoice{ID: "inv_empty"}, want: 0},
	}
	for _, tc := range cases {
		if got := InvoiceTotal(tc.inv); got != tc.want {
			t.Errorf("%s: got %d, want %d", tc.name, got, tc.want)
		}
	}
}
`,
  );
  write(
    'contracts/fee-rounding-cases.json',
    JSON.stringify(
      {
        source: 'gateway settlement export',
        cases: [
          { invoice: 'inv_0041', lines: 12, expected_total: 1247 },
          { invoice: 'inv_0042', lines: 3, expected_total: 604 },
        ],
      },
      null,
      2,
    ) + '\n',
  );

  const git = (...args) => {
    const r = spawnSync('git', args, {
      cwd: root,
      env: {
        ...process.env,
        HOME: stageHome,
        GIT_CONFIG_GLOBAL: '/dev/null',
        GIT_CONFIG_SYSTEM: '/dev/null',
        GIT_AUTHOR_NAME: 'ledger',
        GIT_AUTHOR_EMAIL: 'ledger@example.invalid',
        GIT_COMMITTER_NAME: 'ledger',
        GIT_COMMITTER_EMAIL: 'ledger@example.invalid',
      },
      encoding: 'utf8',
    });
    if (r.status !== 0) throw new Error(`git ${args.join(' ')} -> ${r.stderr || r.stdout}`);
    return r.stdout;
  };
  git('init', '-q', '-b', 'master');
  git('add', '-A');
  git('commit', '-qm', 'ledger: invoicing and fee arithmetic');

  // Now dirty it exactly the way the worker's log says it did.
  write(
    'internal/fees/round.go',
    fs
      .readFileSync(path.join(root, 'internal/fees/round.go'), 'utf8')
      .replace(
        '\tfor _, li := range inv.Lines {\n\t\ttotal += round(li.Fee)\n\t}\n\treturn total',
        '\tfor _, li := range inv.Lines {\n\t\ttotal += li.Fee\n\t}\n\treturn round(total)',
      ),
  );
  write(
    'internal/fees/round_test.go',
    fs
      .readFileSync(path.join(root, 'internal/fees/round_test.go'), 'utf8')
      .replace(
        '\t\t{name: "empty invoice", inv: Invoice{ID: "inv_empty"}, want: 0},\n',
        '\t\t{name: "empty invoice", inv: Invoice{ID: "inv_empty"}, want: 0},\n\t\t{name: "twelve lines, one rounding", inv: twelveLine(), want: 1241},\n',
      ),
  );
  write(
    'contracts/fee-rounding-cases.json',
    fs
      .readFileSync(path.join(root, 'contracts/fee-rounding-cases.json'), 'utf8')
      .replace('"expected_total": 1247', '"expected_total": 1241'),
  );
  return root;
}
