/**
 * The `/supervise` skill: the fleet-supervisor's loop, shipped with the app and
 * installed (idempotently) as a personal Claude Code skill the moment a
 * supervisor session is spawned. Keeping it a real skill — not just system-prompt
 * text — means the user can read and edit it (`~/.claude/skills/supervise/`), and
 * the supervisor invokes it as `/supervise` and re-runs it on a loop.
 *
 * The skill ships with a parsing helper (`fleet.mjs`) installed alongside it.
 * The supervisor runs that script for read-heavy work — fleet status, new
 * conversation turns, a worker's reply — so the raw JSON is parsed
 * deterministically in a subprocess and never enters the supervisor's context.
 * The `mcp__workspacer__*` tools remain the control plane (spawn, message,
 * notify, approve). Nothing here is assumed by the rest of the app.
 */

import * as os from 'os';
import * as path from 'path';
import * as fs from 'fs';

const SKILL_NAME = 'supervise';

const SKILL_BODY = `---
name: supervise
description: Coordinate the Workspacer agent fleet — watch every running agent, summarize what each is doing using cheap summarizer workers, and surface decisions that need a human with full context. Only useful inside a Workspacer supervisor session (requires the mcp__workspacer__* tools).
---

# Supervise the fleet

You are the Workspacer fleet supervisor. Your job is to keep a live picture of
every other agent and to surface anything that needs the human — without doing
their coding for them, and without burning your own context reading raw
transcripts. You offload the reading to a cheap summarizer worker and to a
bundled parsing script.

If you do **not** have the \`mcp__workspacer__*\` tools, stop: this skill does
nothing outside a supervisor session.

## Helpers — parse with the script, act with the tools

A Node helper is installed next to this skill. Run it with Bash; it prints
compact, already-parsed text (no JSON), so use it for READS instead of pulling
raw tool output into your context:

\`\`\`
node "$HOME/.claude/skills/supervise/fleet.mjs" status
node "$HOME/.claude/skills/supervise/fleet.mjs" convo <sessionId> --since <seq>
node "$HOME/.claude/skills/supervise/fleet.mjs" reply <sessionId>
\`\`\`

- \`status\` — one line per session: id, mode, what it's blocked on, context use, cwd.
- \`convo <id> --since <seq>\` — prints \`seq=<latest>\` then only the turns after
  \`<seq>\`, condensed. Omit \`--since\` for the whole conversation.
- \`reply <id>\` — just that session's latest assistant message (how you read a
  worker's digest).

Use the \`mcp__workspacer__*\` tools for ACTIONS: spawn_agent, send_message,
notify, approve, answer. (\`list_agents\` / \`get_conversation\` exist too, but
prefer the script for routine reads.)

## 0. Settings

Call \`get_config\` once and read \`supervisor.summarizerModel\` (fallback
\`sonnet\`) and \`supervisor.pollSeconds\` (fallback \`45\`). Your system prompt also
states these as a fallback if the config call fails.

## 1. Keep one cheap summarizer worker

Spawn a single long-lived digest worker the first time through, then reuse it:

\`\`\`
spawn_agent({
  model: <summarizerModel>,
  mcpFacade: true,          // so the worker can call get_transcript itself
  label: "fleet digest",
  parentSessionId: <your session id>,
  cwd: <any active agent's cwd, or the host cwd>
})
\`\`\`

\`mcpFacade: true\` gives the worker the workspacer tools but does NOT make it a
supervisor — it just reads transcripts and answers you. Reusing one worker keeps
cost down; only spawn another if the first dies.

## 2. Each pass — work incrementally

Keep a per-agent cursor: the last conversation \`seq\` you have digested.

1. Run \`fleet.mjs status\` to see the fleet and who is blocked. Ignore your own
   session and the digest worker.
2. For each agent, run \`fleet.mjs convo <id> --since <last seq>\`. The first line
   is \`seq=<latest>\`; advance your cursor to it. If it prints \`(no new turns)\`,
   skip the agent — nothing changed.
3. Hand the new turns to the digest worker (it does the heavy reading, so it
   costs *its* context, not yours):
   \`\`\`
   send_message(<digestWorkerId>,
     "New turns for session <id>:\\n<paste the convo output>\\n" +
     "Update your running digest and reply with <=3 lines: GOAL / NOW / BLOCKED-ON (or 'not blocked').")
   \`\`\`
   Then read its answer with \`fleet.mjs reply <digestWorkerId>\` (poll until it
   has replied). For a deeper read, instead tell the worker to call
   get_transcript for the session itself.
4. Maintain a short fleet status from those digests. When the user asks "what's
   everyone doing?", answer from this — don't re-read transcripts yourself.

## 3. Decisions — the important part

When \`status\` shows an agent \`blocked=approval:...\` or \`blocked=question\`, it is
waiting on a human. Assemble everything needed to decide and send ONE enriched
notification:

- What it wants to do (the command / diff / the question + options) — from
  \`status\` and \`convo\`, or \`get_snapshot\` for the exact tool input.
- A one-line "why now" from that agent's latest digest.
- Your read on the risk and a recommendation.

\`\`\`
notify({
  title: "<agent label> needs a decision",
  body: "<what it wants to do> — <why> — <your recommendation>. Reply in session:<id>."
})
\`\`\`

Always write a referenced session as \`session:<sessionId>\` so the UI links it.
Don't approve or answer on the human's behalf unless they've told you to.

## 4. Loop + wakes

Run a pass, then schedule the next one ~\`pollSeconds\` apart so you keep watching.
Prefer the \`/loop\` skill if it's available (\`/loop <pollSeconds>s /supervise\`);
otherwise re-invoke \`/supervise\` after each pass. Keep passes cheap: only
re-summarize agents that actually changed (your seq cursors tell you who did),
and lean on the digest worker.

You may also be **woken between passes**: when an agent blocks on a decision,
workspacer sends you a message starting with \`[supervisor]\`. Treat that as a
priority trigger — run a pass immediately, focusing on the named session, and
notify the human with the context + your recommendation.
`;

// The parsing helper. Written deliberately free of backticks, ${...} and
// backslash escapes so it survives verbatim inside this template literal.
const FLEET_SCRIPT = `#!/usr/bin/env node
// fleet.mjs — parsing helper for the /supervise skill. Talks to claudemon's
// local REST API and prints compact, already-parsed text so the supervisor
// never has to reason over raw JSON. Zero dependencies (Node 18+ global fetch).
//
//   node fleet.mjs status                  fleet overview (mode, blocked, ctx, cwd)
//   node fleet.mjs convo <id> [--since N]   latest seq + only the turns after N
//   node fleet.mjs reply <id>               that session's latest assistant message
//
// Override the daemon URL with the CLAUDEMON_API_URL env var.

const BASE = process.env.CLAUDEMON_API_URL || 'http://127.0.0.1:7891';

async function getJSON(p) {
  const res = await fetch(BASE + p);
  if (!res.ok) throw new Error(p + ' -> HTTP ' + res.status);
  return res.json();
}

function trunc(s, n) {
  s = String(s == null ? '' : s).trim();
  return s.length > n ? s.slice(0, n) + '...' : s;
}

function lineOf(it) {
  if (it.kind === 'user_message') return 'user: ' + trunc(it.text, 500);
  if (it.kind === 'assistant_text') return 'assistant: ' + trunc(it.text, 500);
  if (it.kind === 'tool_use') return 'tool: ' + it.name + ' ' + trunc(JSON.stringify(it.input || {}), 200);
  if (it.kind === 'tool_result') return 'result' + (it.is_error ? '(error)' : '') + ': ' + trunc(it.content, 300);
  return null; // usage etc.
}

function blockedOf(p) {
  if (!p) return '-';
  if (p.kind === 'approval') return 'approval:' + (p.tool || '?');
  if (p.kind === 'question') return 'question';
  return p.kind || '?';
}

async function status() {
  const sessions = await getJSON('/sessions');
  if (!Array.isArray(sessions) || sessions.length === 0) { console.log('(no sessions)'); return; }
  for (const s of sessions) {
    let ctx = '-';
    if (s.status_line && s.status_line.context_used_pct != null) ctx = Math.round(s.status_line.context_used_pct) + '%';
    else if (s.usage && s.usage.contextTokens != null) ctx = s.usage.contextTokens + 'tok';
    console.log('session:' + s.session_id + '  mode=' + s.mode + '  blocked=' + blockedOf(s.pending) + '  ctx=' + ctx + '  cwd=' + (s.cwd || '-'));
  }
}

async function convo(id, since) {
  if (!id) throw new Error('convo needs <id>');
  const q = since != null ? ('?since=' + encodeURIComponent(since)) : '';
  const data = await getJSON('/sessions/' + encodeURIComponent(id) + '/conversation' + q);
  console.log('seq=' + (data.seq || 0));
  const items = Array.isArray(data.items) ? data.items : [];
  let n = 0;
  for (const it of items) { const l = lineOf(it); if (l) { console.log(l); n++; } }
  if (n === 0) console.log('(no new turns)');
}

async function reply(id) {
  if (!id) throw new Error('reply needs <id>');
  const data = await getJSON('/sessions/' + encodeURIComponent(id) + '/conversation');
  const items = Array.isArray(data.items) ? data.items : [];
  for (let i = items.length - 1; i >= 0; i--) {
    if (items[i].kind === 'assistant_text') { console.log(items[i].text); return; }
  }
  console.log('(no reply yet)');
}

const argv = process.argv.slice(2);
const cmd = argv[0];
const si = argv.indexOf('--since');
const sinceVal = si >= 0 ? argv[si + 1] : undefined;

(async () => {
  try {
    if (cmd === 'status') await status();
    else if (cmd === 'convo') await convo(argv[1], sinceVal);
    else if (cmd === 'reply') await reply(argv[1]);
    else { console.error('usage: fleet.mjs status | convo <id> [--since N] | reply <id>'); process.exit(2); }
  } catch (e) { console.error(String((e && e.message) || e)); process.exit(1); }
})();
`;

/** Directory the skill (and its helpers) are installed into. */
function skillDir(): string {
  return path.join(os.homedir(), '.claude', 'skills', SKILL_NAME);
}

/**
 * The supervisor's home directory: `~/.workspacer`. A fleet supervisor watches
 * the whole fleet rather than living in any one project, so it opens here — a
 * stable, neutral scratch space — instead of landing in some random agent's
 * repo. Created (with a short README) on first use. Best-effort: if creation
 * fails we fall back to the home dir. Shared by both spawn paths (ipc.ts and
 * hubCapabilities.ts).
 */
export function ensureSupervisorHome(): string {
  const dir = path.join(os.homedir(), '.workspacer');
  try {
    fs.mkdirSync(dir, { recursive: true });
    const readme = path.join(dir, 'README.md');
    if (!fs.existsSync(readme)) {
      fs.writeFileSync(
        readme,
        '# Workspacer supervisor home\n\n' +
          'This directory is the working directory for fleet **supervisor** agents\n' +
          'spawned from Workspacer (Ask the Fleet). They coordinate your other\n' +
          'Claude Code agents via the workspacer MCP tools and use this folder as a\n' +
          'neutral scratch space — notes, digests, etc. Safe to delete; it is\n' +
          'recreated on the next supervisor spawn.\n',
        'utf8',
      );
    }
    return dir;
  } catch {
    return os.homedir();
  }
}

/** Write `file` only if its content changed, to avoid churning the user's files
 *  (and any editor/watcher) on every spawn. Best-effort. */
function writeIfChanged(file: string, content: string): void {
  let current = '';
  try { current = fs.readFileSync(file, 'utf8'); } catch { /* not installed yet */ }
  if (current !== content) fs.writeFileSync(file, content, 'utf8');
}

/**
 * Install the `/supervise` skill (SKILL.md + the fleet.mjs parsing helper) into
 * the user's personal Claude Code skills dir, refreshing them so the supervisor
 * always runs the current version. Best-effort: a failure just means the
 * supervisor falls back to its system prompt / the MCP tools. Safe to call on
 * every supervisor spawn.
 */
export function installSupervisorSkill(): void {
  try {
    const dir = skillDir();
    fs.mkdirSync(dir, { recursive: true });
    writeIfChanged(path.join(dir, 'SKILL.md'), SKILL_BODY);
    writeIfChanged(path.join(dir, 'fleet.mjs'), FLEET_SCRIPT);
  } catch {
    /* installing the skill is best-effort */
  }
}
