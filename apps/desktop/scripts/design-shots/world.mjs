/**
 * The invented world every design shot is taken in.
 *
 * Nothing here is real: the user is "dev", the projects live under
 * /home/dev/work, and the people/companies/repos are made up. The snapshot
 * shapes follow src/renderer/src/types/claudeSession.ts; the bus answers follow
 * the call sites in src/renderer/src/backend/webBackend.ts.
 *
 * `makeWorld({ now, configOverrides })` returns:
 *   config     — the WHOLE config document config.get answers (generated
 *                defaults + the world's directories/projects + overrides)
 *   layout     — the hub layout document (agents/workspaces/panes) /app adopts
 *   snapshots  — live session snapshots (sessions.snapshots / agent.snapshot)
 *   jobs       — the hub's jobs.json (jobs are hub-local)
 *   handlers   — bus method → (params, ctx) => result
 */
import * as fs from 'fs';
import * as path from 'path';
import { REPO, deepMerge } from './stage.mjs';

const DEFAULTS = JSON.parse(
  fs.readFileSync(path.join(REPO, 'services/hub-rs/assets/config-defaults.json'), 'utf8'),
);

export const HOME = '/home/dev';
export const P = {
  atlas: `${HOME}/work/atlas`,
  ledger: `${HOME}/work/ledger`,
  orbit: `${HOME}/work/orbit-web`,
  relay: `${HOME}/work/relay`,
  docs: `${HOME}/work/handbook`,
};

export function makeWorld({ now = Date.now(), configOverrides = {} } = {}) {
  const sec = (ms) => Math.floor(ms / 1000);
  const tool = (id, name, input, status = 'complete', at = now - 600_000, response) => ({
    id,
    name,
    input,
    status,
    startedAt: at,
    completedAt: status === 'running' ? undefined : at + 1400,
    ...(response !== undefined ? { response } : {}),
  });
  const base = (o) => ({
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
    totalToolCalls: 0,
    livePermissionMode: 'default',
    ...o,
  });

  // ── the hero: atlas, mid-conversation, live tool call, plan, workflow ─────
  const ATLAS = base({
    sessionId: 'atlas-7c41',
    cwd: P.atlas,
    label: 'Retire the v1 ingest path',
    ambientState: 'streaming',
    lastActivity: now - 3000,
    totalToolCalls: 86,
    settings: { model: 'claude-opus-4-8', effort: 'high', permissionMode: 'default' },
    conversation: [
      {
        role: 'user',
        content:
          'The v1 ingest path is dead weight — nothing has written through it since the March cutover. Remove it, sweep the call sites, and keep the v2 contract tests green. Don\u2019t touch the schema migration.',
        timestamp: now - 1_500_000,
      },
      {
        role: 'assistant',
        content:
          'I\u2019ll start by mapping every caller of the v1 ingest entry points so the deletion is mechanical rather than exploratory.',
        timestamp: now - 1_480_000,
      },
      {
        role: 'assistant',
        content: '',
        timestamp: now - 1_470_000,
        toolCalls: [
          tool('a1', 'Grep', { pattern: 'ingest\\.v1\\.', path: 'src' }, 'complete', now - 1_470_000,
            'src/pipeline/router.ts:41\nsrc/pipeline/router.ts:88\nsrc/workers/backfill.ts:17\nsrc/api/upload.ts:203\nsrc/api/upload.ts:240'),
          tool('a2', 'Read', { file_path: `${P.atlas}/src/pipeline/router.ts` }, 'complete', now - 1_460_000),
          tool('a3', 'Read', { file_path: `${P.atlas}/src/workers/backfill.ts` }, 'complete', now - 1_455_000),
        ],
      },
      {
        role: 'assistant',
        content:
          'Five call sites across three files. Two of them (`router.ts:41` and `upload.ts:203`) are feature-flagged fallbacks that have been `false` in every environment since March 14, so they can go outright. The backfill worker still imports a v1 type for its job payload — I\u2019ll move that type into `ingest/v2/types.ts` first so nothing breaks mid-sweep.',
        timestamp: now - 1_440_000,
      },
      {
        role: 'assistant',
        content: '',
        timestamp: now - 1_400_000,
        toolCalls: [
          tool('a4', 'Edit', {
            file_path: `${P.atlas}/src/ingest/v2/types.ts`,
            old_string: "export type IngestJob = {\n  id: string;\n  source: SourceRef;\n};",
            new_string:
              "export type IngestJob = {\n  id: string;\n  source: SourceRef;\n  /** Carried over from v1 so backfill keeps its payload shape. */\n  priority?: 'bulk' | 'interactive';\n};",
          }, 'complete', now - 1_400_000),
          tool('a5', 'Edit', {
            file_path: `${P.atlas}/src/pipeline/router.ts`,
            old_string:
              "if (flags.ingestV1Fallback) {\n  return ingestV1.route(req);\n}\nreturn ingestV2.route(req);",
            new_string: 'return ingestV2.route(req);',
          }, 'complete', now - 1_380_000),
          tool('a6', 'Bash', { command: 'pnpm vitest run src/ingest --reporter=dot', description: 'Run ingest tests' },
            'complete', now - 1_300_000, '✓ 142 passed (3.81s)'),
        ],
      },
      {
        role: 'user',
        content: 'nice. does anything outside src still import it? check the scripts dir too',
        timestamp: now - 900_000,
      },
      {
        role: 'assistant',
        content:
          'Good call — `scripts/reprocess.ts` imports `ingestV1.parseManifest` directly. It\u2019s a one-off operator script, but it\u2019s still run monthly per the runbook, so I\u2019m porting it to the v2 parser rather than deleting it.',
        timestamp: now - 860_000,
      },
      {
        role: 'assistant',
        content: '',
        timestamp: now - 820_000,
        toolCalls: [
          tool('a7', 'Glob', { pattern: 'scripts/**/*.ts' }, 'complete', now - 820_000),
          tool('a8', 'Edit', {
            file_path: `${P.atlas}/scripts/reprocess.ts`,
            old_string: "import { parseManifest } from '../src/ingest/v1/manifest';",
            new_string: "import { parseManifest } from '../src/ingest/v2/manifest';",
          }, 'complete', now - 800_000),
          tool('a9', 'Bash', { command: 'git rm -r src/ingest/v1', description: 'Delete the v1 module' },
            'complete', now - 700_000, "rm 'src/ingest/v1/index.ts'\nrm 'src/ingest/v1/manifest.ts'\nrm 'src/ingest/v1/route.ts'"),
        ],
      },
      {
        role: 'assistant',
        content:
          'v1 is gone. Running the full contract suite now — it\u2019s the slow one (~2 min) but it\u2019s the one that would catch a payload-shape regression.',
        timestamp: now - 600_000,
      },
    ],
    activeToolCalls: [
      tool('a10', 'Bash', { command: 'pnpm test:contracts --filter ingest', description: 'Contract suite' }, 'running', now - 45_000),
    ],
    completedToolCalls: [],
    fileChanges: [
      { path: `${P.atlas}/src/ingest/v2/types.ts`, toolName: 'Edit', input: { old_string: 'a\nb\nc\nd', new_string: 'a\nb\nc\nd\ne\nf' }, timestamp: now - 1_400_000 },
      { path: `${P.atlas}/src/pipeline/router.ts`, toolName: 'Edit', input: { old_string: 'a\nb\nc\nd', new_string: 'a' }, timestamp: now - 1_380_000 },
      { path: `${P.atlas}/scripts/reprocess.ts`, toolName: 'Edit', input: { old_string: 'a', new_string: 'b' }, timestamp: now - 800_000 },
    ],
    plan: {
      steps: [
        { content: 'Map every v1 ingest caller', status: 'completed' },
        { content: 'Move IngestJob into v2 types', status: 'completed' },
        { content: 'Remove feature-flagged fallbacks', status: 'completed' },
        { content: 'Port scripts/reprocess.ts to v2', status: 'completed' },
        { content: 'Run the contract suite', status: 'in_progress', activeForm: 'Running the contract suite' },
        { content: 'Write the PR description', status: 'pending' },
      ],
      updatedAt: now - 40_000,
    },
    subagents: [
      {
        id: 'sub-a1',
        type: 'Explore',
        status: 'complete',
        startedAt: now - 1_200_000,
        completedAt: now - 1_150_000,
        description: 'Find v1 references outside src/',
        tokens: 22_400,
        toolCalls: 11,
      },
      {
        id: 'sub-a2',
        type: 'general-purpose',
        status: 'running',
        startedAt: now - 70_000,
        description: 'Draft the PR description from the diff',
        tokens: 9_800,
        toolCalls: 4,
        lastToolName: 'Read',
        lastToolSummary: 'src/pipeline/router.ts',
      },
    ],
    workflows: [
      {
        runId: 'run-ingest-audit',
        name: 'contract-sweep',
        description: 'Fan out one checker per downstream consumer of the ingest payload',
        status: 'running',
        startedAt: now - 260_000,
        totalTokens: 184_000,
        totalToolCalls: 63,
        totalCostUSD: 1.71,
        phases: [{ title: 'Survey consumers' }, { title: 'Verify payloads' }],
        agents: [
          { id: 'wa-1', label: 'billing-consumer', phaseTitle: 'Survey consumers', status: 'done', tokens: 31_000, toolCalls: 12, durationMs: 48_000, resultPreview: 'reads priority — compatible' },
          { id: 'wa-2', label: 'search-indexer', phaseTitle: 'Survey consumers', status: 'done', tokens: 27_000, toolCalls: 9, durationMs: 39_000, resultPreview: 'ignores priority — compatible' },
          { id: 'wa-3', label: 'analytics-export', phaseTitle: 'Verify payloads', status: 'running', tokens: 44_000, toolCalls: 17, startedAt: now - 90_000, lastToolName: 'Bash', lastToolSummary: 'pnpm test exports' },
          { id: 'wa-4', label: 'partner-webhooks', phaseTitle: 'Verify payloads', status: 'failed', tokens: 18_000, toolCalls: 8, durationMs: 41_000, resultPreview: 'fixture missing priority field' },
          { id: 'wa-5', label: 'archive-replay', phaseTitle: 'Verify payloads', status: 'queued', tokens: 0, toolCalls: 0 },
        ],
      },
    ],
    usage: {
      model: 'claude-opus-4-8',
      contextTokens: 88_000,
      contextLimit: 200_000,
      totalInputTokens: 1_240_000,
      totalOutputTokens: 61_000,
      costUSD: 4.12,
    },
    statusLine: {
      modelDisplay: 'Opus 4.8',
      contextUsedPct: 44,
      contextWindowSize: 200_000,
      totalInputTokens: 1_240_000,
      totalOutputTokens: 61_000,
      costUSD: 4.12,
      fiveHourPct: 38,
      fiveHourResetsAt: sec(now + 2.4 * 3_600_000),
      sevenDayPct: 57,
      sevenDayResetsAt: sec(now + 3 * 86_400_000),
      capabilities: {
        mcpServers: 3, skills: 5, plugins: 2, agents: 3, memoryFiles: 2,
        inventory: {
          claudeCodeVersion: '2.4.1',
          mcpServers: [
            { name: 'github', status: 'connected', source: 'user', estTokens: 4_200 },
            { name: 'linear', status: 'connected', source: 'user', estTokens: 2_900 },
            { name: 'postgres-ro', status: 'failed', source: 'project' },
          ],
          skills: [
            { name: 'release-notes', source: 'project', path: `${P.atlas}/.claude/skills/release-notes/SKILL.md`, description: 'Draft release notes from merged PRs', bytes: 3_400, estTokens: 850 },
            { name: 'contract-tests', source: 'project', path: `${P.atlas}/.claude/skills/contract-tests/SKILL.md`, description: 'Run and triage the payload contract suite', bytes: 5_100, estTokens: 1_275 },
            { name: 'pdf', source: 'built-in' },
            { name: 'review', source: 'user', path: `/home/dev/.claude/skills/review/SKILL.md`, description: 'Self-review a diff before opening a PR', bytes: 2_200, estTokens: 550 },
            { name: 'flaky-hunter', source: 'team-tools', description: 'Reproduce and bisect flaky tests', bytes: 4_000, estTokens: 1_000 },
          ],
          agents: [
            { name: 'Explore', source: 'built-in' },
            { name: 'general-purpose', source: 'built-in' },
            { name: 'migration-reviewer', source: 'project', description: 'Checks schema migrations for lock risk', bytes: 1_900, estTokens: 475 },
          ],
          plugins: [
            { name: 'team-tools', source: 'github:example/team-tools' },
            { name: 'workspacer', source: 'built-in' },
          ],
          memoryFiles: [
            { name: 'CLAUDE.md', path: `${P.atlas}/CLAUDE.md`, source: 'project', bytes: 9_600, estTokens: 2_400 },
            { name: 'CLAUDE.md', path: '/home/dev/.claude/CLAUDE.md', source: 'user', bytes: 1_800, estTokens: 450 },
          ],
          tools: ['Bash', 'Read', 'Edit', 'Write', 'Glob', 'Grep', 'Task', 'TodoWrite', 'WebFetch', 'WebSearch'],
          slashCommands: ['/review', '/compact', '/clear', '/release-notes'],
        },
      },
    },
  });

  // ── ledger: blocked on a permission prompt ────────────────────────────────
  const LEDGER = base({
    sessionId: 'ledger-2f9d',
    cwd: P.ledger,
    label: 'Reconcile fee rounding',
    ambientState: 'waiting_approval',
    lastActivity: now - 50_000,
    totalToolCalls: 23,
    conversation: [
      {
        role: 'user',
        content: 'Fee totals are drifting against the gateway by a few cents on large invoices. Find out why.',
        timestamp: now - 700_000,
      },
      {
        role: 'assistant',
        content:
          'The drift is per-line rounding: each line item rounds to a cent, and the sum of twelve roundings is not the rounding of the sum. I changed `round.go` to round once on the invoice total. Now I need to replay last month\u2019s invoices against the gateway sandbox to confirm.',
        timestamp: now - 120_000,
        toolCalls: [
          tool('l1', 'Read', { file_path: `${P.ledger}/internal/fees/round.go` }),
          tool('l2', 'Edit', {
            file_path: `${P.ledger}/internal/fees/round.go`,
            old_string: 'for _, li := range inv.Lines {\n\ttotal += round(li.Fee)\n}',
            new_string: 'for _, li := range inv.Lines {\n\ttotal += li.Fee\n}\ntotal = round(total)',
          }),
        ],
      },
    ],
    pendingApproval: {
      toolName: 'Bash',
      toolInput: {
        command: 'go run ./cmd/replay --corpus invoices-2026-10.jsonl --gateway sandbox',
        description: 'Replay October invoices against the gateway sandbox',
      },
      timestamp: now - 50_000,
    },
    usage: { model: 'claude-sonnet-5', contextTokens: 41_000, contextLimit: 200_000, totalInputTokens: 210_000, totalOutputTokens: 9_000, costUSD: 0.71 },
    statusLine: { modelDisplay: 'Sonnet 5', contextUsedPct: 21, costUSD: 0.71 },
  });

  // ── orbit-web: blocked on a question ──────────────────────────────────────
  const ORBIT = base({
    sessionId: 'orbit-91ab',
    cwd: P.orbit,
    label: 'Dark-mode pass on settings',
    ambientState: 'waiting_input',
    lastActivity: now - 140_000,
    totalToolCalls: 31,
    conversation: [
      {
        role: 'user',
        content: 'Settings pages look washed out in dark mode. Fix the contrast without inventing new tokens.',
        timestamp: now - 1_100_000,
      },
      {
        role: 'assistant',
        content: 'Auditing every surface token the settings routes use.',
        timestamp: now - 1_050_000,
        toolCalls: [
          tool('o1', 'Glob', { pattern: 'src/routes/settings/**/*.tsx' }, 'complete', now - 1_040_000),
          tool('o2', 'Grep', { pattern: 'var\\(--gray-200\\)', path: 'src' }, 'complete', now - 1_000_000,
            'src/routes/settings/Profile.tsx:14\nsrc/routes/settings/Billing.tsx:22\nsrc/routes/settings/Team.tsx:9'),
          tool('o3', 'Read', { file_path: `${P.orbit}/src/styles/tokens.css` }, 'complete', now - 980_000),
        ],
      },
      {
        role: 'assistant',
        content:
          'Three surfaces use `--gray-200` as a background, which is near-white in both themes. The token sheet already has two dark-aware surfaces that would fit:\n\n| Token | Light | Dark |\n|---|---|---|\n| `--surface-raised` | `#ffffff` | `#1e2228` |\n| `--surface-sunken` | `#f4f5f7` | `#14171b` |\n\nBefore I swap them I need one decision from you.',
        timestamp: now - 150_000,
      },
    ],
    pendingQuestions: [
      {
        question: 'Which token should the settings cards sit on in dark mode?',
        header: 'Surface token',
        options: [
          { label: '--surface-raised', description: 'Matches the dashboard cards; one step lighter than the page.' },
          { label: '--surface-sunken', description: 'Reads as inset; better for dense forms.' },
          { label: 'Keep the page background', description: 'Separate cards with borders only.' },
        ],
      },
    ],
    usage: { model: 'claude-opus-4-8', contextTokens: 52_000, contextLimit: 200_000, totalInputTokens: 300_000, totalOutputTokens: 14_000, costUSD: 1.33 },
    statusLine: { modelDisplay: 'Opus 4.8', contextUsedPct: 26, costUSD: 1.33 },
  });

  // ── relay: done, waiting for review with a sizeable diff ──────────────────
  const RELAY = base({
    sessionId: 'relay-55e0',
    cwd: P.relay,
    provider: 'codex',
    label: 'Add retry budget to webhooks',
    ambientState: 'idle',
    lastActivity: now - 780_000,
    totalToolCalls: 47,
    conversation: [
      { role: 'user', content: 'Webhook retries hammer slow receivers. Add a per-endpoint retry budget.', timestamp: now - 2_400_000 },
      {
        role: 'assistant',
        content: 'Looking at how the delivery worker schedules retries today.',
        timestamp: now - 2_350_000,
        toolCalls: [
          tool('r1', 'Read', { file_path: `${P.relay}/internal/delivery/worker.go` }, 'complete', now - 2_340_000),
          tool('r2', 'Write', { file_path: `${P.relay}/internal/delivery/budget.go`, content: 'package delivery\n\n// Budget is a per-endpoint token bucket for retries.\ntype Budget struct {\n\tcapacity int\n\trefill   time.Duration\n}\n' }, 'complete', now - 2_000_000),
          tool('r3', 'Edit', { file_path: `${P.relay}/internal/delivery/worker.go`, old_string: 'if attempt < maxAttempts {\n\tw.requeue(job)\n}', new_string: 'if attempt < maxAttempts && w.budget.Take(job.Endpoint) {\n\tw.requeue(job)\n} else {\n\tw.park(job.Endpoint)\n}' }, 'complete', now - 1_700_000),
          tool('r4', 'Bash', { command: 'go test ./internal/delivery/...' }, 'complete', now - 900_000, 'ok  \trelay/internal/delivery\t2.114s'),
        ],
      },
      {
        role: 'assistant',
        content:
          'Done. Each endpoint now gets a token-bucket retry budget (10 retries / 5 min, configurable). Exhausted endpoints are parked and surfaced in the admin view. Tests added for the bucket refill and for the park/unpark transitions; all 318 pass.',
        timestamp: now - 780_000,
      },
    ],
    fileChanges: [
      { path: `${P.relay}/internal/delivery/budget.go`, toolName: 'Write', input: { content: new Array(84).fill('x').join('\n') }, timestamp: now - 1_500_000 },
      { path: `${P.relay}/internal/delivery/worker.go`, toolName: 'Edit', input: { old_string: new Array(12).fill('a').join('\n'), new_string: new Array(31).fill('b').join('\n') }, timestamp: now - 1_400_000 },
      { path: `${P.relay}/internal/delivery/budget_test.go`, toolName: 'Write', input: { content: new Array(120).fill('t').join('\n') }, timestamp: now - 1_300_000 },
      { path: `${P.relay}/config/defaults.yaml`, toolName: 'Edit', input: { old_string: 'a', new_string: 'a\nb\nc' }, timestamp: now - 1_200_000 },
    ],
    usage: { model: 'gpt-5.4', contextTokens: 63_000, contextLimit: 272_000, totalInputTokens: 880_000, totalOutputTokens: 41_000, costUSD: 2.08 },
    statusLine: { modelDisplay: 'gpt-5.4', contextUsedPct: 23, costUSD: 2.08 },
  });

  // ── handbook: plain idle ──────────────────────────────────────────────────
  const DOCS = base({
    sessionId: 'docs-0c3a',
    cwd: P.docs,
    provider: 'opencode',
    label: 'Onboarding guide refresh',
    ambientState: 'idle',
    lastActivity: now - 5_400_000,
    totalToolCalls: 9,
    conversation: [
      { role: 'user', content: 'Refresh the onboarding guide for the new deploy flow.', timestamp: now - 6_000_000 },
      { role: 'assistant', content: 'Updated `onboarding.md` and the deploy diagram. Ready for a read-through.', timestamp: now - 5_400_000 },
    ],
    usage: { model: 'kimi-k2', contextTokens: 12_000, contextLimit: 128_000, totalInputTokens: 40_000, totalOutputTokens: 3_000, costUSD: 0.09 },
  });

  const snapshots = [ATLAS, LEDGER, ORBIT, RELAY, DOCS];

  // ── layout: one workspace per agent ───────────────────────────────────────
  let paneSeq = 0;
  const pane = (type, extra = {}) => ({ id: `pane-${type}-${++paneSeq}`, type, title: type, ...extra });
  const agentPane = (s) =>
    pane('claude', { title: s.label, transport: 'stream', attachSessionId: s.sessionId, expectHistory: true, provider: s.provider });
  const ws = (id, s, panes) => {
    const ps = panes ?? [agentPane(s)];
    return {
      id,
      name: s.label,
      nameSetByUser: true,
      cwd: s.cwd,
      provider: s.provider,
      sessionId: s.sessionId,
      tabs: [{ id: `${id}-tab`, title: 'Main', panes: ps, activePaneId: ps[0].id }],
      activeTabId: `${id}-tab`,
    };
  };
  const layout = {
    agents: [
      ws('agent-atlas', ATLAS),
      ws('agent-ledger', LEDGER),
      ws('agent-orbit', ORBIT),
      ws('agent-relay', RELAY),
      ws('agent-docs', DOCS),
    ],
    activeAgentId: 'agent-atlas',
  };

  const config = deepMerge(
    deepMerge(DEFAULTS, {
      // One-time notices that would otherwise cover a populated screen. The
      // first-run shot turns these back on explicitly.
      onboardingDismissed: true,
      ui: { commandLayerAnnounced: true },
      directories: {
        favourites: [P.atlas, P.ledger, P.orbit],
        recent: [P.relay, P.docs, HOME],
      },
    }),
    configOverrides,
  );

  // ── git ───────────────────────────────────────────────────────────────────
  const ATLAS_DIFFS = {
    'src/pipeline/router.ts': `diff --git a/src/pipeline/router.ts b/src/pipeline/router.ts
index 3b1c2d0..9f8e7a1 100644
--- a/src/pipeline/router.ts
+++ b/src/pipeline/router.ts
@@ -1,9 +1,7 @@
 import { flags } from '../flags';
-import * as ingestV1 from '../ingest/v1';
 import * as ingestV2 from '../ingest/v2';
 import type { IngestRequest, RouteResult } from './types';

 /**
  * Route an ingest request to the pipeline that owns its source.
- * v1 remains reachable behind \`ingestV1Fallback\` until the cutover settles.
  */
 export function route(req: IngestRequest): RouteResult {
@@ -38,12 +36,7 @@ export function route(req: IngestRequest): RouteResult {
   if (req.source.kind === 'archive') {
     return archiveRoute(req);
   }
-
-  if (flags.ingestV1Fallback) {
-    metrics.increment('ingest.v1.fallback');
-    return ingestV1.route(req);
-  }
   return ingestV2.route(req);
 }

@@ -85,7 +78,6 @@ function archiveRoute(req: IngestRequest): RouteResult {
   const manifest = req.source.manifest;
-  if (manifest.version === 1) return ingestV1.route(req);
   return ingestV2.route({ ...req, priority: 'bulk' });
 }
`,
    'src/ingest/v2/types.ts': `diff --git a/src/ingest/v2/types.ts b/src/ingest/v2/types.ts
index 1a2b3c4..5d6e7f8 100644
--- a/src/ingest/v2/types.ts
+++ b/src/ingest/v2/types.ts
@@ -12,6 +12,8 @@ export type SourceRef = {
 export type IngestJob = {
   id: string;
   source: SourceRef;
+  /** Carried over from v1 so backfill keeps its payload shape. */
+  priority?: 'bulk' | 'interactive';
 };

 export type IngestResult =
`,
    'scripts/reprocess.ts': `diff --git a/scripts/reprocess.ts b/scripts/reprocess.ts
index 0f0e0d0..1c1b1a1 100644
--- a/scripts/reprocess.ts
+++ b/scripts/reprocess.ts
@@ -1,5 +1,5 @@
 #!/usr/bin/env tsx
-import { parseManifest } from '../src/ingest/v1/manifest';
+import { parseManifest } from '../src/ingest/v2/manifest';
 import { enqueue } from '../src/workers/queue';

 const [, , manifestPath] = process.argv;
`,
  };
  const ATLAS_STATUS = {
    branch: 'retire-ingest-v1',
    upstream: 'origin/retire-ingest-v1',
    ahead: 2,
    behind: 0,
    files: [
      { path: 'src/pipeline/router.ts', staged: ' ', unstaged: 'M' },
      { path: 'src/ingest/v2/types.ts', staged: 'M', unstaged: ' ' },
      { path: 'scripts/reprocess.ts', staged: ' ', unstaged: 'M' },
      { path: 'src/ingest/v1/index.ts', staged: 'D', unstaged: ' ' },
      { path: 'src/ingest/v1/manifest.ts', staged: 'D', unstaged: ' ' },
      { path: 'src/ingest/v1/route.ts', staged: 'D', unstaged: ' ' },
    ],
  };
  const NUMSTAT = [
    { path: 'src/pipeline/router.ts', added: 0, deleted: 8 },
    { path: 'src/ingest/v2/types.ts', added: 2, deleted: 0 },
    { path: 'scripts/reprocess.ts', added: 1, deleted: 1 },
    { path: 'src/ingest/v1/index.ts', added: 0, deleted: 34 },
    { path: 'src/ingest/v1/manifest.ts', added: 0, deleted: 112 },
    { path: 'src/ingest/v1/route.ts', added: 0, deleted: 71 },
  ];
  const LOG = [
    ['e41f9a2', 'ingest: move IngestJob priority into v2 types'],
    ['b77c013', 'pipeline: drop the v1 fallback flag check'],
    ['4a0d6ee', 'workers: backfill reads v2 payloads'],
    ['91c2b48', 'api: upload streams straight to v2'],
    ['0d3e5f1', 'flags: retire ingestV1Fallback'],
    ['7b8a9c0', 'Merge pull request #412 from atlas/contract-tests'],
  ].map(([hash, subject], i) => ({ hash: hash + '3f2a1b0c9d8e7f6a5b4c3d2e1f0a9b8c7d6e5f4a'.slice(0, 33), subject, authoredAt: sec(now - (i + 1) * 5_400_000) }));

  // ── analytics / usage ─────────────────────────────────────────────────────
  const day = (i) => new Date(now - i * 86_400_000).toISOString().slice(0, 10);
  const daily = [3.1, 5.4, 2.2, 7.9, 6.3, 1.4, 0.8, 4.6, 8.8, 9.7, 5.2, 6.1, 3.3, 7.4];
  const analyticsSummary = {
    totals: { sessions: 143, costUSD: 412.37, inputTokens: 98_400_000, outputTokens: 3_120_000, toolCalls: 11_842, durationMs: 412 * 3_600_000, workflowRuns: 19, unrecordedSessions: 0 },
    byDay: daily.map((c, i) => ({ key: day(daily.length - 1 - i), sessions: Math.round(c * 1.7), costUSD: c, tokens: Math.round(c * 240_000) })),
    byProject: [
      { key: P.atlas, sessions: 41, costUSD: 151.2, tokens: 36_000_000 },
      { key: P.ledger, sessions: 33, costUSD: 98.4, tokens: 22_000_000 },
      { key: P.orbit, sessions: 29, costUSD: 77.9, tokens: 18_400_000 },
      { key: P.relay, sessions: 25, costUSD: 61.0, tokens: 15_100_000 },
      { key: P.docs, sessions: 15, costUSD: 23.9, tokens: 7_000_000 },
    ],
    byModel: [
      { key: 'claude-opus-4-8', sessions: 61, costUSD: 244.1, tokens: 51_000_000 },
      { key: 'claude-sonnet-5', sessions: 48, costUSD: 101.3, tokens: 33_000_000 },
      { key: 'gpt-5.4', sessions: 24, costUSD: 58.6, tokens: 12_400_000 },
      { key: 'kimi-k2', sessions: 10, costUSD: 8.4, tokens: 5_100_000 },
    ],
    byProvider: [
      { key: 'claude', sessions: 109, costUSD: 345.4, tokens: 84_000_000 },
      { key: 'codex', sessions: 24, costUSD: 58.6, tokens: 12_400_000 },
      { key: 'opencode', sessions: 10, costUSD: 8.4, tokens: 5_100_000 },
    ],
  };
  const projectsCycle = [P.atlas, P.ledger, P.orbit, P.relay, P.docs];
  const titles = [
    'Retire the v1 ingest path', 'Reconcile fee rounding', 'Dark-mode pass on settings', 'Add retry budget to webhooks',
    'Onboarding guide refresh', 'Flaky upload test on CI', 'Split the billing worker', 'Bump TypeScript to 5.9',
    'Audit public API error codes', 'Cache the pricing table', 'Fix the pagination cursor', 'Trim the docker image',
  ];
  const analyticsRecent = titles.map((t, i) => ({
    sessionId: `hist-${i}`,
    cwd: projectsCycle[i % 5],
    agentName: t,
    provider: i % 4 === 3 ? 'codex' : 'claude',
    model: i % 4 === 3 ? 'gpt-5.4' : i % 2 ? 'claude-sonnet-5' : 'claude-opus-4-8',
    gitBranch: 'main',
    startedAt: new Date(now - (i + 1) * 9_000_000).toISOString(),
    endedAt: new Date(now - (i + 1) * 9_000_000 + 3_000_000).toISOString(),
    durationMs: 3_000_000,
    inputTokens: 400_000 + i * 31_000,
    outputTokens: 18_000 + i * 900,
    costUSD: +(1.2 + ((i * 1.7) % 6)).toFixed(2),
    peakContext: 90_000,
    toolCalls: 40 + i * 3,
    messageCount: 18 + i,
  }));
  const usageReport = {
    generated_at: sec(now),
    evaluated_at: sec(now),
    valid_until: sec(now + 300_000),
    providers: [
      {
        provider: 'claude',
        accounts: [
          {
            account: '',
            label: 'dev@example.com',
            is_default: true,
            source: 'statusline',
            observed_at: sec(now - 30_000),
            fresh: true,
            windows: {
              five_hour: { used_percent: { state: 'ok', value: 38 }, resets_at: sec(now + 2.4 * 3_600_000), window_minutes: 300, is_current: true, pace: { known: true, state: 'on_pace', usedPct: 38, expectedPct: 52, elapsedPct: 52 } },
              seven_day: { used_percent: { state: 'ok', value: 57 }, resets_at: sec(now + 3 * 86_400_000), window_minutes: 10_080, is_current: true, pace: { known: true, state: 'ahead', usedPct: 57, expectedPct: 57, elapsedPct: 57 } },
            },
          },
        ],
      },
      {
        provider: 'codex',
        accounts: [
          {
            account: '',
            label: 'ChatGPT Pro',
            is_default: true,
            source: 'rate_limits',
            observed_at: sec(now - 600_000),
            fresh: true,
            windows: {
              five_hour: { used_percent: { state: 'ok', value: 12 }, resets_at: sec(now + 4 * 3_600_000), window_minutes: 300, is_current: true },
              seven_day: { used_percent: { state: 'ok', value: 31 }, resets_at: sec(now + 5 * 86_400_000), window_minutes: 10_080, is_current: true },
            },
          },
        ],
      },
    ],
  };

  const recentRow = (i, o) => ({
    sessionId: `hist-${i}`,
    provider: 'claude',
    mode: 'stopped',
    transport: 'stream',
    archived: false,
    startedAt: o.updatedAt - 3_100_000,
    name: o.title,
    model: 'claude-opus-4-8',
    costUSD: +(0.8 + i * 0.63).toFixed(2),
    billedTokens: 220_000 + i * 41_000,
    ...o,
  });
  const recent = [
    recentRow(5, { cwd: P.atlas, title: 'Flaky upload test on CI', updatedAt: now - 7_200_000 }),
    recentRow(6, { cwd: P.ledger, title: 'Split the billing worker', updatedAt: now - 30_000_000, model: 'claude-sonnet-5' }),
    recentRow(7, { cwd: P.orbit, title: 'Bump TypeScript to 5.9', updatedAt: now - 86_400_000, provider: 'codex', model: 'gpt-5.4' }),
    recentRow(8, { cwd: P.relay, title: 'Audit public API error codes', updatedAt: now - 2 * 86_400_000 }),
    recentRow(9, { cwd: P.atlas, title: 'Cache the pricing table', updatedAt: now - 3 * 86_400_000, model: 'claude-sonnet-5' }),
    recentRow(10, { cwd: P.ledger, title: 'Fix the pagination cursor', updatedAt: now - 4 * 86_400_000 }),
    recentRow(11, { cwd: P.docs, title: 'Trim the docker image', updatedAt: now - 6 * 86_400_000, provider: 'opencode', model: 'kimi-k2' }),
    recentRow(12, { cwd: P.relay, title: 'Webhook signature rotation', updatedAt: now - 9 * 86_400_000, archived: true }),
  ];
  const history = recent.map((r, i) => ({
    ...r,
    hidden: false,
    onSidebar: false,
    endedAt: r.updatedAt,
    firstPrompt: `${r.title} — see the linked issue for context.`,
    userPrompts: 3 + (i % 4),
    toolCalls: 30 + i * 7,
    transcriptCopy: true,
  }));

  // Fleet Manager dispatch history (Recent agents pane / task inspector).
  const isoAt = (ms) => new Date(ms).toISOString();
  const attempt = (o) => ({
    kind: 'fresh',
    acceptedAt: isoAt(o.t),
    observedAt: isoAt(o.t + 60_000),
    lifecycle: 'ended',
    stale: false,
    live: false,
    resultContract: 'valid',
    requestedProvider: 'claude',
    provider: 'claude',
    requestedModel: 'claude-sonnet-5',
    reportedModel: 'claude-sonnet-5',
    role: 'implementer',
    worktree: { requested: true, allocated: true, fallback: false, branch: `wks/${o.dispatchId}` },
    metrics: { wallMs: 14 * 60_000, inputTokens: 380_000, outputTokens: 21_000, costUSD: 1.42, context: { used: 64_000, limit: 200_000, observedAt: isoAt(o.t) } },
    ...o,
  });
  const dispatchHistory = {
    available: true,
    currentOwnerSessionId: 'mgr-1',
    tasks: [
      {
        taskId: 'task-ingest', ownerSessionId: 'mgr-1', ownerLabel: 'Fleet Manager', projectCwd: P.atlas,
        title: 'Retire the v1 ingest path', createdAt: isoAt(now - 2 * 3_600_000),
        attempts: [
          attempt({ dispatchId: 'd-1', sessionId: 'w-1', stage: 'scout', t: now - 7_000_000, executionCwd: P.atlas, role: 'scout', metrics: { wallMs: 6 * 60_000, inputTokens: 120_000, outputTokens: 6_000, costUSD: 0.31 } }),
          attempt({ dispatchId: 'd-2', sessionId: 'atlas-7c41', stage: 'implement', afterDispatchId: 'd-1', t: now - 1_600_000, executionCwd: P.atlas, lifecycle: 'running', live: true, resultContract: 'absent', requestedModel: 'claude-opus-4-8', reportedModel: 'claude-opus-4-8' }),
        ],
      },
      {
        taskId: 'task-fees', ownerSessionId: 'mgr-1', ownerLabel: 'Fleet Manager', projectCwd: P.ledger,
        title: 'Reconcile fee rounding against the gateway', createdAt: isoAt(now - 3 * 3_600_000),
        attempts: [
          attempt({ dispatchId: 'd-3', sessionId: 'ledger-2f9d', stage: 'implement', t: now - 800_000, executionCwd: P.ledger, lifecycle: 'needs-decision', live: true, resultContract: 'absent' }),
        ],
      },
      {
        taskId: 'task-retry', ownerSessionId: 'mgr-1', ownerLabel: 'Fleet Manager', projectCwd: P.relay,
        title: 'Per-endpoint webhook retry budget', createdAt: isoAt(now - 6 * 3_600_000),
        attempts: [
          attempt({ dispatchId: 'd-4', sessionId: 'w-4', stage: 'implement', t: now - 5_000_000, executionCwd: P.relay, provider: 'codex', requestedProvider: 'codex', requestedModel: 'gpt-5.4', reportedModel: 'gpt-5.4' }),
          attempt({ dispatchId: 'd-5', sessionId: 'w-5', stage: 'review', afterDispatchId: 'd-4', t: now - 2_600_000, executionCwd: P.relay, role: 'reviewer', resultContract: 'escalated' }),
          attempt({ dispatchId: 'd-6', sessionId: 'relay-55e0', stage: 'fix', afterDispatchId: 'd-5', kind: 'retry', retryOfDispatchId: 'd-4', t: now - 1_900_000, executionCwd: P.relay, provider: 'codex', requestedProvider: 'codex', requestedModel: 'gpt-5.4', reportedModel: 'gpt-5.4', lifecycle: 'idle' }),
        ],
      },
      {
        taskId: 'task-docs', ownerSessionId: 'mgr-1', ownerLabel: 'Fleet Manager', projectCwd: P.docs,
        title: 'Refresh the onboarding guide', createdAt: isoAt(now - 26 * 3_600_000),
        attempts: [
          attempt({ dispatchId: 'd-7', sessionId: 'docs-0c3a', stage: 'implement', t: now - 25 * 3_600_000, executionCwd: P.docs, provider: 'opencode', requestedProvider: 'opencode', requestedModel: 'kimi-k2', reportedModel: 'kimi-k2' }),
          attempt({ dispatchId: 'd-8', sessionId: 'w-8', stage: 'validate', afterDispatchId: 'd-7', t: now - 24 * 3_600_000, executionCwd: P.docs, role: 'validator' }),
        ],
      },
    ],
  };

  const savedLayouts = [
    {
      id: 'layout-morning', name: 'Morning triage', createdAt: isoAt(now - 12 * 86_400_000),
      agents: [
        { name: 'atlas', cwd: P.atlas, tabs: [{ title: 'Main', panes: [{ type: 'claude', title: 'Agent' }, { type: 'terminal', title: 'Shell' }] }] },
        { name: 'relay', cwd: P.relay, tabs: [{ title: 'Main', panes: [{ type: 'claude', title: 'Agent' }] }] },
      ],
    },
    {
      id: 'layout-release', name: 'Release week', createdAt: isoAt(now - 30 * 86_400_000),
      agents: [
        { name: 'ledger', cwd: P.ledger, tabs: [{ title: 'Main', panes: [{ type: 'claude', title: 'Agent' }, { type: 'review', title: 'Review' }] }] },
        { name: 'orbit-web', cwd: P.orbit, tabs: [{ title: 'Main', panes: [{ type: 'claude', title: 'Agent' }, { type: 'browser', title: 'Preview', url: 'http://localhost:5173' }] }] },
        { name: 'handbook', cwd: P.docs, tabs: [{ title: 'Main', panes: [{ type: 'claude', title: 'Agent' }] }] },
      ],
    },
  ];

  const routingView = JSON.parse(
    fs.readFileSync(path.join(REPO, 'apps/desktop/tests/fixtures/routing-preferences-view.json'), 'utf8'),
  );
  const routingValidation = { catalogChecked: true, valid: true, catalogPending: false, issues: [], changedPaths: [] };

  const library = [
    { id: 'lib-standup', scope: 'global', title: 'Standup summary', kind: 'prompt', body: 'Summarise what changed in this repo since yesterday, grouped by area.' },
    { id: 'lib-review', scope: 'global', title: 'Pre-PR self review', kind: 'prompt', body: 'Review the current diff for correctness bugs before I open a PR.' },
    { id: 'lib-triage', scope: 'global', title: 'Issue triage sweep', kind: 'prompt', body: 'Triage open issues labelled needs-triage.' },
    { id: 'lib-flaky', scope: 'project', title: 'Chase a flaky test', kind: 'prompt', body: 'Reproduce {{test}} 50x and find the race.' },
  ];

  const tree = {
    [P.atlas]: [
      ['src', true], ['scripts', true], ['tests', true], ['package.json', false], ['README.md', false], ['tsconfig.json', false],
    ],
    [`${P.atlas}/src`]: [['api', true], ['ingest', true], ['pipeline', true], ['workers', true], ['flags.ts', false], ['index.ts', false]],
  };
  const listDir = (p) =>
    (tree[p] ?? [['README.md', false]]).map(([name, isDir]) => ({ name, path: `${p}/${name}`, isDirectory: isDir, isDir, type: isDir ? 'dir' : 'file' }));

  const FILES = {
    'README.md': '# atlas\n\nIngest pipeline for partner data drops.\n\n## Layout\n\n- `src/ingest/v2` — the only ingest path\n- `src/pipeline` — routing\n- `src/workers` — backfill and reprocess\n',
  };

  let replayed = false;
  const handlers = {
    'sessions.conversation': () => ({ seq: 1, items: [] }),
    'sessions.recent': () => recent,
    'sessions.list': () => [],
    'sessions.save': () => ({ ok: true }),
    'sessions.load': () => null,
    'sessions.delete': () => ({ ok: true }),
    'sessions.history': () => ({ sessions: history }),
    'desktop.dispatchHistoryRead': () => dispatchHistory,
    'desktop.providerReadiness': () => ({ state: 'responding', checkedAt: now - 120_000 }),
    'desktop.agentRuntimeStatus': () => ({ claudemon: 'ready', hub: 'ready', facade: 'ready' }),
    'ui.fonts': () => [],
    'routing.preferences.get': () => routingView,
    'routing.preferences.validate': () => ({ status: 'valid', view: routingView, validation: routingValidation }),
    'routing.preferences.save': () => ({ status: 'applied', view: routingView, validation: routingValidation }),
    'routing.preferences.reset': () => ({ status: 'applied', view: routingView, validation: routingValidation }),
    'routing.preview': (req) => {
      const capability = routingView.effective.roles[req.role] ?? 'standard';
      const prof = routingView.effective.profiles[req.profile || routingView.effective.activeProfile] ?? {};
      const a = prof[capability] ?? { provider: 'claude', model: 'claude-sonnet-5' };
      return { role: req.role, profile: routingView.effective.activeProfile, provider: a.provider, model: a.model, effort: a.effort || '', capability, baseCapability: capability, fresh: !!a.fresh, eligible: true, capped: false, mode: 'normal', reason: ['Selected from the applied policy. No agent was launched.'], observedAt: now, usageState: 'ok' };
    },
    'library.list': () => library,
    'providers.checkAll': () => [
      { provider: 'claude', found: true, resolvedPath: '/usr/local/bin/claude' },
      { provider: 'codex', found: true, resolvedPath: '/usr/local/bin/codex' },
      { provider: 'opencode', found: true, resolvedPath: '/usr/local/bin/opencode' },
      { provider: 'pi', found: false, resolvedPath: '' },
    ],
    'providers.listModels': () => [{ id: 'gpt-5.4', label: 'gpt-5.4', default: true }],
    'claude.listModels': () => ({
      defaultModel: 'opus',
      aliases: [
        { value: 'claude-opus-4-8', label: 'Opus 4.8' },
        { value: 'claude-sonnet-5', label: 'Sonnet 5' },
      ],
      seen: [],
    }),
    'claude.sessionsForDir': () => [],
    'claude.approve': () => ({ ok: true }),
    'claude.answer': () => ({ ok: true }),
    'claude.signal': () => ({ ok: true }),
    'claude.gate': () => ({ ok: true }),
    'claude.setPermissionMode': () => ({ ok: true }),
    'claude.setModel': () => ({ ok: true }),
    'claude.setEffort': () => ({ ok: true }),
    'agents.sendMessage': () => ({ ok: true, mode: 'queued' }),
    'agents.spawn': () => ({ sessionId: 'spawned-1' }),
    'layouts.list': () => savedLayouts,
    'layouts.save': () => ({ ok: true }),
    'layouts.delete': () => ({ ok: true }),
    'federation.peers': () => [],
    'app.getCwd': () => P.atlas,
    'app.supervisorHome': () => HOME,
    'fs.listDir': (p) => listDir(p.path ?? p.dir ?? p.cwd),
    'fs.listEntries': (p) => listDir(p.path ?? p.dir ?? p.cwd),
    'fs.read': (p) => {
      const rel = String(p.path ?? '').split('/').pop();
      return FILES[rel] ?? `// ${p.path}\n`;
    },
    'fs.watch': () => ({ ok: true }),
    'fs.unwatch': () => ({ ok: true }),
    'git.status': (p) => (p.cwd === P.atlas ? ATLAS_STATUS : { branch: 'main', upstream: 'origin/main', ahead: 0, behind: 0, files: [] }),
    'git.diff': (p) => ({ diff: ATLAS_DIFFS[p.path] ?? Object.values(ATLAS_DIFFS).join('') }),
    'git.numstat': () => ({ files: NUMSTAT }),
    'git.log': () => ({ commits: LOG }),
    'git.commitDiff': () => ({ diff: Object.values(ATLAS_DIFFS).join('') }),
    'git.commitNumstat': () => ({ files: NUMSTAT.slice(0, 3) }),
    'analytics.summary': () => analyticsSummary,
    'analytics.recent': () => analyticsRecent,
    'usage.report': () => usageReport,
    'plugins.tools': () => [],
    'terminals.create': () => ({ sessionId: 'term-1' }),
    'sessions.attachTerminal': (p, ctx) => {
      // attach makes claudemon replay its ring buffer; a resize re-attaches, so
      // replay only once per terminal or the screen prints twice.
      if (p.sessionId === 'term-1' && !replayed) {
        replayed = true;
        setTimeout(() => ctx.publish('pty.bytes.term-1', Buffer.from(TERMINAL_SCREEN).toString('base64')), 200);
      }
      return { ok: true };
    },
    'sessions.terminalKeepalive': () => ({ ok: true }),
    'sessions.terminalResize': () => ({ ok: true }),
    'sessions.terminalInput': () => ({ ok: true }),
    'sessions.detachTerminal': () => ({ ok: true }),
    'search.project': () => ({ results: [] }),
  };

  const iso = (ms) => Math.floor(ms / 1000);
  const jobs = {
    jobs: [
      {
        id: 'job-deps', name: 'Nightly dependency audit', enabled: true,
        trigger: { kind: 'daily', at: '02:30' },
        action: { kind: 'spawn', spawn: { cwd: P.atlas, prompt: 'Audit dependencies for new advisories and open a PR for safe bumps.', provider: 'claude' } },
        createdAt: iso(now - 20 * 86_400_000), updatedAt: iso(now - 3 * 86_400_000),
      },
      {
        id: 'job-triage', name: 'Triage new issues', enabled: true,
        trigger: { kind: 'daily', at: '08:15', days: [1, 2, 3, 4, 5] },
        action: { kind: 'spawn', spawn: { cwd: P.relay, prompt: 'Label and route issues opened since the last run.', provider: 'codex' } },
        createdAt: iso(now - 12 * 86_400_000), updatedAt: iso(now - 12 * 86_400_000),
      },
      {
        id: 'job-changelog', name: 'Weekly changelog draft', enabled: false,
        trigger: { kind: 'daily', at: '16:00', days: [5] },
        action: { kind: 'spawn', spawn: { cwd: P.docs, prompt: 'Draft the weekly changelog from merged PRs.', provider: 'claude' } },
        createdAt: iso(now - 40 * 86_400_000), updatedAt: iso(now - 9 * 86_400_000),
      },
      {
        id: 'job-proposal', name: 'Re-run flaky upload test hourly', enabled: false,
        proposedBy: 'Fleet Manager',
        trigger: { kind: 'manual' },
        action: { kind: 'spawn', spawn: { cwd: P.atlas, prompt: 'Run the upload test 20x and report flake rate.', provider: 'claude' } },
        createdAt: iso(now - 3_600_000), updatedAt: iso(now - 3_600_000),
      },
    ],
  };
  const jobsHistory = {
    runs: [
      { jobId: 'job-deps', startedAt: iso(now - 11 * 3_600_000), finishedAt: iso(now - 10.6 * 3_600_000), status: 'ok', detail: 'spawned atlas agent' },
      { jobId: 'job-deps', startedAt: iso(now - 35 * 3_600_000), finishedAt: iso(now - 34.7 * 3_600_000), status: 'ok', detail: 'spawned atlas agent' },
      { jobId: 'job-triage', startedAt: iso(now - 5 * 3_600_000), finishedAt: iso(now - 4.9 * 3_600_000), status: 'error', detail: 'codex: rate limited' },
    ],
  };

  return { config, layout, snapshots, jobs, jobsHistory, handlers, P, now };
}

const TERMINAL_SCREEN = [
  '\x1b[1;32mdev@workstation\x1b[0m:\x1b[1;34m~/work/atlas\x1b[0m$ git status -sb',
  '## retire-ingest-v1...origin/retire-ingest-v1 [ahead 2]',
  'M  src/ingest/v2/types.ts',
  'D  src/ingest/v1/index.ts',
  'D  src/ingest/v1/manifest.ts',
  'D  src/ingest/v1/route.ts',
  ' M scripts/reprocess.ts',
  ' M src/pipeline/router.ts',
  '\x1b[1;32mdev@workstation\x1b[0m:\x1b[1;34m~/work/atlas\x1b[0m$ pnpm vitest run src/ingest',
  '',
  ' \x1b[1;46;30m RUN \x1b[0m \x1b[36mv3.2.4\x1b[0m ~/work/atlas',
  '',
  ' \x1b[32m✓\x1b[0m src/ingest/v2/manifest.test.ts \x1b[2m(38 tests)\x1b[0m \x1b[33m412ms\x1b[0m',
  ' \x1b[32m✓\x1b[0m src/ingest/v2/route.test.ts \x1b[2m(57 tests)\x1b[0m \x1b[33m688ms\x1b[0m',
  ' \x1b[32m✓\x1b[0m src/ingest/v2/types.test.ts \x1b[2m(12 tests)\x1b[0m \x1b[33m91ms\x1b[0m',
  ' \x1b[32m✓\x1b[0m src/pipeline/router.test.ts \x1b[2m(35 tests)\x1b[0m \x1b[33m520ms\x1b[0m',
  '',
  ' \x1b[2m Test Files \x1b[0m \x1b[1;32m4 passed\x1b[0m\x1b[2m (4)\x1b[0m',
  ' \x1b[2m      Tests \x1b[0m \x1b[1;32m142 passed\x1b[0m\x1b[2m (142)\x1b[0m',
  ' \x1b[2m   Duration \x1b[0m 3.81s',
  '',
  '\x1b[1;32mdev@workstation\x1b[0m:\x1b[1;34m~/work/atlas\x1b[0m$ ',
].join('\r\n');
