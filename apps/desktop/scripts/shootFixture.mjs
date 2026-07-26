/**
 * Fabricated fleet for the staged screenshot run (`scripts/shoot.mjs`).
 *
 * A fresh stage profile has no Claude credentials, so agents cannot be spawned.
 * Sessions are *observed* into existence instead, the same way a real Claude
 * Code run produces them — which is why this works without auth:
 *
 *   1. A transcript JSONL is written to the stage's Claude projects dir at
 *      `$HOME/.claude/projects/<cwd with / : \ -> ->/<session-id>.jsonl`
 *      (`session/transcript.rs::encoded_cwd`). This is the ONLY source of
 *      assistant/user message text — hooks never carry it.
 *   2. `POST :7890/hook/session_start` with `transcript_path` binds the session
 *      to that file. `read_at()` treats the hook's path as authoritative, so no
 *      cwd guessing is involved.
 *   3. `POST :7890/statusline` attaches model, context %, and cost — the only
 *      channel that carries them.
 *   4. `POST :7890/hook/pre_tool` leaves a tool call parked as a pending
 *      approval, which is what puts a card in the "needs you" state.
 *
 * Everything lands on the staged loopback inside the network namespace, so none
 * of it can reach a live daemon.
 */
import * as fs from 'fs';
import * as path from 'path';

const HOOK = 'http://127.0.0.1:7890';

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

/** Real Claude transcript shape: `type` is the role, text hangs off `message.content`. */
const userLine = (text, ts) =>
  JSON.stringify({ type: 'user', message: { role: 'user', content: text }, timestamp: ts });
const asstLine = (text, ts) =>
  JSON.stringify({
    type: 'assistant',
    message: { role: 'assistant', content: [{ type: 'text', text }] },
    timestamp: ts,
  });

const iso = (msAgo) => new Date(Date.now() - msAgo).toISOString();

const AGENTS = [
  {
    id: 'stage-workspacer',
    cwd: '/home/dev/work/workspacer',
    model: 'Fable 5',
    ctx: 32,
    cost: 46.24,
    lines: [
      userLine('final prep pass before we flip public', iso(9 * 60000)),
      asstLine('Running the secret scanner over the full history first.', iso(8 * 60000)),
      asstLine(
        'All three prep items are done — history is clean, downloads are wired, and the landing page is ready to flip public.',
        iso(4 * 60000),
      ),
    ],
  },
  {
    id: 'stage-prep',
    cwd: '/home/dev/work/prep',
    model: 'Opus 4.8',
    ctx: 17,
    cost: 0.21,
    lines: [
      userLine('where is the release pipeline at?', iso(5 * 60000)),
      asstLine('Checking the release pipeline before I summarize.', iso(3 * 60000)),
    ],
  },
  {
    id: 'stage-rivet',
    cwd: '/home/dev/work/rivet',
    model: 'Opus 4.8',
    ctx: 8,
    cost: 0.34,
    approval: { tool: 'Bash', input: { command: 'psql prod -c "select count(*) from coverage"' } },
    lines: [
      userLine('verify the coverage numbers', iso(7 * 60000)),
      asstLine(
        'I need to run the coverage query against the prod snapshot to be sure.',
        iso(4 * 60000),
      ),
    ],
  },
  {
    id: 'stage-recon',
    cwd: '/home/dev/work/recon',
    model: 'Opus 4.8',
    ctx: 3,
    cost: 0.05,
    lines: [
      userLine('map the cache layout', iso(2 * 60000)),
      asstLine('Mapping the cache layout — three tiers, symbol index is the hot one.', iso(40000)),
    ],
  },
];

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

    // Claude Code's statusLine payload shape — the only carrier of context %,
    // cumulative cost, and the rate-limit windows.
    await post(`${HOOK}/statusline`, {
      session_id: a.id,
      model: { display_name: a.model },
      workspace: { current_dir: a.cwd },
      cost: { total_cost_usd: a.cost },
      context: { used_pct: a.ctx },
    });

    if (a.approval) {
      await post(`${HOOK}/hook/pre_tool`, {
        session_id: a.id,
        cwd: a.cwd,
        tool_name: a.approval.tool,
        tool_input: a.approval.input,
      });
    }
  }
  return AGENTS.map((a) => a.id);
}
