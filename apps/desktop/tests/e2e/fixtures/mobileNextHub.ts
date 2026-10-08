import { test } from '@playwright/test';
import { buildRustHubFixture } from './rustHub';
/**
 * Test rig for /m-next (services/hub-rs/assets/web/m-next).
 *
 * Same shape as mobileHub.ts: the real hub binary on a scratch port with a
 * scratch home, and a fake capability provider on /bus answering the methods
 * the client calls. The difference is the data: /m-next is built for the Rust
 * hub's sparse snapshots, so conversations, subagent transcripts and task
 * logs come from their own methods (sessions.conversation,
 * sessions.subagentConversation, sessions.taskOutput) the way claudemon
 * serves them, and the host's kept-open set is a real native-settings.json in
 * the scratch XDG_CONFIG_HOME that the hub's own `sessions.kept` reads.
 *
 * Hub-owned state stays hub-owned: jobs are proposed through jobs.propose,
 * the archive is the hub's sessionArchive.
 */
import { spawn, type ChildProcess } from 'child_process';
import * as fs from 'fs';
import * as path from 'path';
import {
  assertNoLiveStateHandles,
  assertScratchEnv,
  assertScratchPath,
  freePort,
  makeScratchDir,
  removeScratchDir,
  scratchEnv,
  withBuildLock,
} from './scratchState';

export const HOST_TOKEN = 'test-next-host-token';
export const VIEW_TOKEN = 'test-next-view-token';

export interface CallRecord {
  method: string;
  params: any;
}
export interface NextHub {
  url: string;
  calls: CallRecord[];
  callsTo(method: string): CallRecord[];
  snapshots: Map<string, any>;
  pushSnapshot(snap: any): void;
  /** Pristine fixture rows, an empty call log and no archived sessions. */
  reset(): Promise<void>;
  stop(): Promise<void>;
}

async function waitForHealth(url: string, timeoutMs = 15000): Promise<void> {
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

export async function startNextHub(): Promise<NextHub> {
  test.setTimeout(600_000);
  const bin = withBuildLock(buildRustHubFixture);
  const dir = makeScratchDir('wks-mnext-e2e');
  const env = scratchEnv(dir);
  assertScratchEnv(env);
  const scratch = (...p: string[]) => assertScratchPath(path.join(dir, ...p), p.join('/'));
  fs.mkdirSync(path.join(dir, 'config', 'workspacer'), { recursive: true });
  fs.mkdirSync(path.join(dir, 'config', 'workspacer-hub'), { recursive: true });
  // The native client's settings on this "host": it had `paused` open when it
  // closed, under some hub identity. `ended` was never kept.
  fs.writeFileSync(
    scratch('config', 'workspacer', 'native-settings.json'),
    JSON.stringify({
      text_size: 15,
      kept_open: { 'ws://127.0.0.1:7811/bus': { paused: Date.now() - 86_400_000 } },
    }),
  );
  const tokensFile = path.join(dir, 'tokens.json');
  fs.writeFileSync(
    tokensFile,
    JSON.stringify([
      { token: VIEW_TOKEN, scope: 'view', label: 'glance', created: new Date().toISOString() },
    ]),
  );
  const port = await freePort();
  const url = `http://127.0.0.1:${port}`;
  const proc: ChildProcess = spawn(
    bin,
    [
      '--mode',
      'browser',
      '--root',
      dir,
      '--listen',
      `127.0.0.1:${port}`,
      '--token',
      HOST_TOKEN,
      '--tokens-file',
      tokensFile,
      '--layout-file',
      path.join(dir, 'layout.json'),
      '--push-dir',
      path.join(dir, 'push'),
      '--peers-file',
      scratch('config', 'workspacer', 'peers.json'),
      '--jobs-file',
      scratch('config', 'workspacer-hub', 'jobs.json'),
    ],
    { stdio: ['pipe', 'pipe', 'pipe'], env },
  );
  proc.stderr?.on('data', (b) => {
    const s = String(b);
    if (/panic|fatal/i.test(s)) console.error('[hub]', s.trim());
  });
  await waitForHealth(url);
  assertNoLiveStateHandles(proc.pid!);

  const snapshots = new Map<string, any>();
  const calls: CallRecord[] = [];
  let logLines = 0;
  let config = structuredClone(CONFIG);
  const seed = () => {
    snapshots.clear();
    for (const s of sessionsFixture()) snapshots.set(s.sessionId, s);
    logLines = 4;
    config = structuredClone(CONFIG);
  };
  seed();

  const ws = new WebSocket(`ws://127.0.0.1:${port}/bus?token=${HOST_TOKEN}`);
  await new Promise<void>((resolve, reject) => {
    ws.addEventListener('open', () => resolve());
    ws.addEventListener('error', () => reject(new Error('provider socket failed')));
  });
  const METHODS = [
    'sessions.snapshots',
    'sessions.snapshot',
    'sessions.conversation',
    'sessions.subagentConversation',
    'sessions.recent',
    'sessions.taskOutput',
    'sessions.taskStop',
    'agents.sendMessage',
    'agents.spawn',
    'claude.approve',
    'claude.answer',
    'claude.signal',
    'claude.setPermissionMode',
    'claude.setEffort',
    'claude.setModel',
    'claude.listModels',
    'claude.handoffSummaryBrief',
    'claude.handoffAgentBrief',
    'claude.handoffBrief',
    'config.get',
    'config.save',
    'library.list',
    'providers.listModels',
    'providers.checkAll',
    'usage.report',
    'git.status',
    'git.diff',
    'fs.listDir',
  ];
  ws.send(JSON.stringify({ op: 'register', methods: METHODS }));
  const reply = (id: string, result: any) => ws.send(JSON.stringify({ op: 'result', id, result }));
  const publish = (snap: any) =>
    ws.send(
      JSON.stringify({
        op: 'publish',
        event: { type: 'agent.snapshot', source: 'e2e', data: snap },
      }),
    );
  const own = new Map<string, (r: any) => void>();
  let ownSeq = 0;
  const callHub = (method: string, params: any) =>
    new Promise<any>((resolve) => {
      const id = 'p' + ++ownSeq;
      own.set(id, resolve);
      ws.send(JSON.stringify({ op: 'call', id, method, params }));
    });

  ws.addEventListener('message', (ev: MessageEvent) => {
    let f: any;
    try {
      f = JSON.parse(String(ev.data));
    } catch {
      return;
    }
    if ((f.op === 'result' || f.op === 'error') && own.has(f.id)) {
      own.get(f.id)!(f.op === 'result' ? f.result : { error: f.error });
      own.delete(f.id);
      return;
    }
    if (f.op !== 'call') return;
    const p = f.params ?? {};
    calls.push({ method: f.method, params: p });
    switch (f.method) {
      case 'sessions.snapshots':
        // The fleet list ages stopped sessions out; `paused` is only kept.
        return reply(
          f.id,
          [...snapshots.values()].filter((s) => s.sessionId !== 'paused'),
        );
      case 'sessions.snapshot':
        return reply(f.id, snapshots.get(p.sessionId) ?? null);
      case 'sessions.conversation': {
        const items = CONVERSATIONS[p.sessionId] ?? [];
        const since = typeof p.sinceSeq === 'number' ? p.sinceSeq : 0;
        return reply(f.id, { seq: items.length, items: items.slice(since) });
      }
      case 'sessions.subagentConversation':
        return reply(f.id, {
          session_id: p.sessionId,
          agent_id: p.agentId,
          seq: CHILD_ITEMS.length,
          first_seq: 1,
          items: CHILD_ITEMS,
        });
      case 'sessions.taskOutput': {
        if (p.taskId !== 'task-shell') {
          return reply(f.id, {
            task_id: p.taskId,
            session_id: p.sessionId,
            offset: 0,
            next_offset: 28,
            size: 28,
            text: '3 passed (38s)\nall green\n',
            done: true,
            running: false,
            status: 'completed',
          });
        }
        // A following log: each read finds one more line.
        const all = LOG_LINES.slice(0, logLines).join('');
        logLines = Math.min(LOG_LINES.length, logLines + 1);
        const offset = typeof p.offset === 'number' ? p.offset : 0;
        return reply(f.id, {
          task_id: 'task-shell',
          session_id: p.sessionId,
          offset,
          next_offset: all.length,
          size: all.length,
          text: all.slice(offset),
          done: false,
          running: true,
          status: 'running',
        });
      }
      case 'sessions.recent':
        return reply(f.id, RECENT);
      case 'config.get':
        return reply(f.id, config);
      case 'config.save':
        if (p.projects) config.projects = p.projects;
        return reply(f.id, config);
      case 'library.list':
        return reply(f.id, [
          {
            id: 'lib-1',
            title: 'Standup',
            kind: 'prompt',
            body: 'Summarise what changed since yesterday.',
          },
        ]);
      case 'providers.checkAll':
        return reply(f.id, [
          { provider: 'claude', found: true },
          { provider: 'codex', found: true },
        ]);
      case 'providers.listModels':
        return reply(f.id, [
          { id: 'gpt-5.6-sol', label: 'GPT-5.6 Sol' },
          { id: 'gpt-5.6-luna', label: 'GPT-5.6 Luna' },
        ]);
      case 'claude.listModels':
        return reply(f.id, {
          defaultModel: 'sonnet',
          aliases: [
            { value: 'claude-sonnet-5', label: 'Sonnet 5' },
            { model: 'claude-opus-4-8', value: 'claude-opus-4-8[1m]', label: 'Opus 4.8 (1M)' },
          ],
          seen: [],
        });
      case 'claude.handoffSummaryBrief':
      case 'claude.handoffAgentBrief':
      case 'claude.handoffBrief':
        return reply(f.id, {
          ok: true,
          path: '/workspaces/ledger/.workspacer/handoffs/2026-10-08-migrate.md',
          markdown: '# Handoff',
        });
      case 'agents.spawn': {
        const id =
          p.resumeSessionId || `spawned-${calls.filter((c) => c.method === 'agents.spawn').length}`;
        const base = snapshots.get(p.resumeSessionId) ?? {};
        const snap = {
          ...base,
          sessionId: id,
          provider: p.provider,
          cwd: p.cwd,
          transport: 'stream',
          label: p.label ?? base.label ?? 'New session',
          status: 'active',
          mode: 'input',
          ambientState: 'idle',
          lastActivity: Date.now(),
          settings: { model: p.model, effort: p.effort, permissionMode: p.permissionMode },
          sparse: true,
          pendingApproval: null,
          pendingQuestions: null,
        };
        snapshots.set(id, snap);
        reply(f.id, { sessionId: id, ...(p.message ? { messageQueued: true } : {}) });
        return publish(snap);
      }
      case 'usage.report':
        return reply(f.id, usageReport());
      case 'git.status':
        return reply(f.id, {
          branch: 'docs/installer',
          files: [
            { path: 'docs/quickstart.md', unstaged: 'M' },
            { path: 'docs/install.md', unstaged: 'M' },
          ],
        });
      case 'git.diff':
        return reply(f.id, {
          diff: '--- a/docs/quickstart.md\n+++ b/docs/quickstart.md\n@@ -1,3 +1,3 @@\n-npm run setup\n+curl -fsSL https://example.test/install.sh | sh\n wks doctor\n',
        });
      case 'fs.listDir':
        return reply(f.id, {
          path: p.path || '/workspaces',
          parent: '/',
          home: '/workspaces',
          dirs: ['orbital', 'ledger'],
        });
      default:
        return reply(f.id, { ok: true });
    }
  });

  // A job an agent proposed: waits for approval.
  await callHub('jobs.propose', {
    id: 'nightly-deps',
    name: 'Nightly dependency check',
    proposedBy: 'Fleet Manager',
    trigger: { kind: 'daily', at: '03:00' },
    action: {
      kind: 'spawn',
      spawn: {
        cwd: '/workspaces/orbital',
        prompt: 'Check for outdated dependencies and open a PR.',
        provider: 'claude',
      },
    },
  });

  return {
    url,
    calls,
    callsTo: (m) => calls.filter((c) => c.method === m),
    snapshots,
    pushSnapshot(snap) {
      snapshots.set(snap.sessionId, snap);
      publish(snap);
    },
    async reset() {
      seed();
      // The archive is hub-owned state: restore every fixture row through it.
      for (const id of snapshots.keys())
        await callHub('sessionArchive.set', { sessionId: id, archived: false });
      calls.length = 0;
    },
    async stop() {
      try {
        ws.close();
      } catch {
        /* gone */
      }
      proc.kill('SIGKILL');
      await new Promise((r) => setTimeout(r, 100));
      removeScratchDir(dir);
    },
  };
}

// ══ fixtures ══════════════════════════════════════════════════════════════
const NOW = Date.now();
const iso = (msAgo: number) => new Date(NOW - msAgo).toISOString();
const base = (o: any) => ({
  status: 'active',
  transport: 'stream',
  sparse: true,
  pendingApproval: null,
  pendingQuestions: null,
  subagents: [],
  lastActivity: NOW - 60_000,
  ...o,
});

export function sessionsFixture() {
  return [
    base({
      sessionId: 'appr',
      label: 'Retire the v1 ingest path',
      cwd: '/workspaces/atlas',
      provider: 'claude',
      mode: 'approval',
      ambientState: 'waiting_approval',
      settings: { model: 'claude-opus-4-8', permissionMode: 'default' },
      statusLine: { modelDisplay: 'Opus 4.8', contextUsedPct: 74, contextWindowSize: 200000 },
      usage: {
        model: 'claude-opus-4-8',
        contextTokens: 148000,
        contextLimit: 200000,
        costUSD: 3.1,
      },
      pendingApproval: {
        toolName: 'Bash',
        toolInput: {
          command: 'psql "$STAGING_URL" -f migrations/0042_drop_v1_ingest.sql',
          description: 'Drop the v1 ingest tables in staging',
        },
      },
    }),
    base({
      sessionId: 'ques',
      label: 'Reconcile the fee rounding',
      cwd: '/workspaces/ledger',
      provider: 'codex',
      mode: 'question',
      ambientState: 'waiting_input',
      settings: { model: 'gpt-5.6-sol', permissionMode: 'ask' },
      usage: { model: 'gpt-5.6-sol', contextTokens: 37000, contextLimit: 100000 },
      pendingQuestions: [
        {
          header: 'Migration',
          question:
            'The 138 already-issued invoices still carry per-line rounding. What should happen to them?',
          options: [
            {
              label: 'Leave them as issued',
              description: "They're final documents. Only new invoices round on the total.",
            },
            {
              label: 'Reissue corrected invoices',
              description: 'Customers get a replacement PDF and a notice.',
            },
            {
              label: 'Credit the difference',
              description: "Add a credit line to each customer's next invoice.",
            },
          ],
        },
        {
          header: 'Notice',
          question: 'Should customers be told about the change?',
          options: [{ label: 'Email them' }, { label: 'No notice' }],
        },
      ],
    }),
    base({
      sessionId: 'work',
      label: 'Rewrite the getting-started guide',
      cwd: '/workspaces/orbital',
      provider: 'claude',
      mode: 'responding',
      ambientState: 'streaming',
      lastActivity: NOW - 2000,
      settings: { model: 'claude-sonnet-5', effort: 'high', permissionMode: 'default' },
      statusLine: {
        modelDisplay: 'Sonnet 5',
        contextUsedPct: 41,
        contextWindowSize: 200000,
        costUSD: 1.84,
      },
      usage: {
        model: 'claude-sonnet-5',
        contextTokens: 82000,
        contextLimit: 200000,
        costUSD: 1.84,
      },
      background_tasks: 2,
      background_task_list: [
        {
          id: 'task-shell',
          taskType: 'local_bash',
          status: 'running',
          description: 'npm run docs:dev',
          startedAt: NOW - 252_000,
          hasOutput: true,
        },
        {
          id: 'task-agent',
          taskType: 'local_agent',
          status: 'running',
          description: 'Explore: find stale links',
          startedAt: NOW - 63_000,
          subagentId: 'sub-links',
        },
        {
          id: 'task-test',
          taskType: 'local_bash',
          status: 'completed',
          description: 'npx playwright test quickstart',
          startedAt: NOW - 400_000,
          endedAt: NOW - 362_000,
          hasOutput: true,
        },
      ],
      subagents: [
        {
          id: 'sub-links',
          toolUseId: 'tu-agent',
          description: 'Explore: find stale links',
          type: 'Explore',
          status: 'running',
          model: 'claude-haiku-4-5',
          startedAt: NOW - 63_000,
          tokens: 9400,
          lastToolName: 'Grep',
          lastToolSummary: 'wiki.example',
        },
      ],
    }),
    base({
      sessionId: 'shots',
      parentSessionId: 'work',
      label: 'Screenshot the new flow',
      cwd: '/workspaces/orbital',
      provider: 'codex',
      mode: 'responding',
      ambientState: 'streaming',
      settings: { model: 'gpt-5.6-luna', permissionMode: 'ask' },
    }),
    base({
      sessionId: 'paused',
      label: 'Migrate settings schema',
      cwd: '/workspaces/ledger',
      provider: 'claude',
      status: 'ended',
      mode: 'stopped',
      ambientState: 'idle',
      lastActivity: NOW - 86_400_000,
      settings: { model: 'claude-opus-4-8', permissionMode: 'default' },
      statusLine: { modelDisplay: 'Opus 4.8', contextUsedPct: 78, contextWindowSize: 800000 },
      usage: { model: 'claude-opus-4-8', contextTokens: 624000, contextLimit: 800000 },
      promptCache: {
        ttlSeconds: 3600,
        lastRequestAt: NOW - 3 * 3_600_000,
        expiresAt: NOW - 2 * 3_600_000,
        contextTokens: 624000,
        estimated: false,
        coldCostUSD: 6.24,
        warmCostUSD: 0.312,
      },
    }),
    base({
      sessionId: 'ended',
      label: 'Fix the VAT export',
      cwd: '/workspaces/ledger',
      provider: 'claude',
      status: 'ended',
      mode: 'stopped',
      ambientState: 'idle',
      lastActivity: NOW - 3 * 86_400_000,
      settings: { model: 'claude-sonnet-5', permissionMode: 'default' },
    }),
    base({
      sessionId: 'ready',
      label: 'Benchmark cold start',
      cwd: '/workspaces/orbital',
      provider: 'claude',
      mode: 'input',
      ambientState: 'idle',
      settings: { model: 'claude-sonnet-5', permissionMode: 'default' },
    }),
  ];
}

const CARD = JSON.stringify({
  v: 1,
  title: 'Docs check',
  fallback: 'Every command in the new quickstart ran on a clean container.',
  bodyHtml:
    '<p>Every command in the new quickstart ran on a clean container.</p><table><tr><th>Command</th><th>Result</th></tr><tr><td>install.sh</td><td>Passes</td></tr><tr><td>wks doctor</td><td>Passes</td></tr></table><script>window.__pwned = 1</script><img src="https://example.test/x.png" onerror="window.__pwned = 2">',
  actions: [
    { kind: 'fill_composer', label: 'Publish', text: 'Publish the new quickstart.' },
    { kind: 'view_diff', label: 'quickstart.md', path: 'docs/quickstart.md' },
  ],
});

export const CONVERSATIONS: Record<string, any[]> = {
  work: [
    {
      kind: 'user_message',
      text: 'The quickstart still says `npm run setup`. Rewrite it around the new installer.',
      timestamp: iso(300_000),
    },
    {
      kind: 'assistant_text',
      text: 'Rewrote **Getting started** around the installer.\n\n## What changed\n\n- Install is one step\n- The first agent section uses New agent\n\n| Command | Result |\n|---|---|\n| install.sh | Passes |\n| wks doctor | Passes |\n\n```rust\nfn main() {\n    let installed = check(\"wks\");\n    println!(\"{installed}\");\n}\n```',
      timestamp: iso(290_000),
    },
    {
      kind: 'tool_use',
      id: 'tu-edit',
      name: 'Edit',
      input: {
        file_path: '/workspaces/orbital/docs/quickstart.md',
        old_string: 'npm run setup',
        new_string: 'curl … | sh\nwks doctor',
      },
      timestamp: iso(280_000),
    },
    {
      kind: 'tool_result',
      tool_use_id: 'tu-edit',
      content: 'ok',
      is_error: false,
      timestamp: iso(279_000),
    },
    {
      kind: 'tool_use',
      id: 'tu-bash',
      name: 'Bash',
      input: { command: 'wks doctor' },
      timestamp: iso(270_000),
    },
    {
      kind: 'tool_result',
      tool_use_id: 'tu-bash',
      content: 'all checks passed',
      is_error: false,
      timestamp: iso(268_000),
    },
    {
      kind: 'assistant_text',
      text: '```wks-html-card\n' + CARD + '\n```',
      timestamp: iso(260_000),
    },
    {
      kind: 'tool_use',
      id: 'tu-agent',
      name: 'Agent',
      input: {
        description: 'Explore: find stale links',
        prompt: 'Find links that still point at the old wiki.',
      },
      timestamp: iso(64_000),
    },
  ],
  paused: [
    { kind: 'user_message', text: 'Keep going with the v3 schema.', timestamp: iso(86_500_000) },
    {
      kind: 'assistant_text',
      text: 'The v3 schema and its writer are in. Next is the backfill.',
      timestamp: iso(86_450_000),
    },
  ],
  ques: [
    {
      kind: 'assistant_text',
      text: 'Fixed: rounding happens once on the invoice total.',
      timestamp: iso(200_000),
    },
  ],
  appr: [
    {
      kind: 'assistant_text',
      text: 'No reader is left. Dropping the staging tables next.',
      timestamp: iso(50_000),
    },
  ],
};

const CHILD_ITEMS = [
  {
    kind: 'user_message',
    text: 'Find links that still point at the old wiki.',
    timestamp: iso(63_000),
  },
  {
    kind: 'tool_use',
    id: 'c1',
    name: 'Grep',
    input: { pattern: 'wiki.example' },
    timestamp: iso(60_000),
  },
  {
    kind: 'tool_result',
    tool_use_id: 'c1',
    content: '3 matches',
    is_error: false,
    timestamp: iso(59_000),
  },
  {
    kind: 'assistant_text',
    text: 'Three links still point at the old wiki: install.md, faq.md and the README.',
    timestamp: iso(40_000),
  },
];

const LOG_LINES = [
  '  VITE v6.3.1  ready in 412 ms\n',
  '  ➜  Local:   http://localhost:5173/\n',
  '21:33:58 [vite] page reload docs/quickstart.md\n',
  '21:34:11 ✓ built quickstart in 88 ms\n',
  '21:34:21 ✓ built install in 61 ms\n',
  '21:34:40 GET /quickstart 200 4ms\n',
  '21:34:41 GET /assets/shot-1.png 200 2ms\n',
];

const RECENT = [
  {
    sessionId: 'old-1',
    provider: 'codex',
    cwd: '/workspaces/orbital',
    title: 'Draft release notes',
    updatedAt: NOW - 3 * 86_400_000,
  },
];

const CONFIG = {
  projects: {
    '/workspaces/orbital': { favourite: true, lastOpened: NOW - 1000 },
    '/workspaces/ledger': { lastOpened: NOW - 5000 },
  },
  directories: { favourites: [], recent: ['/workspaces/atlas'] },
  agents: { defaultProvider: 'claude' },
};

function usageReport() {
  const now = Math.floor(Date.now() / 1000);
  const w = (pct: number) => ({
    used_percent: { state: 'ok', value: pct },
    resets_at: now + 7200,
    is_current: true,
  });
  return {
    providers: [
      {
        provider: 'claude',
        accounts: [
          { label: 'default', is_default: true, fresh: true, windows: { five_hour: w(42) } },
        ],
      },
      {
        provider: 'codex',
        accounts: [
          { label: 'default', is_default: true, fresh: true, windows: { five_hour: w(93) } },
        ],
      },
    ],
  };
}
