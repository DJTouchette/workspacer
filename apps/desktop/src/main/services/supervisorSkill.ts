/**
 * The `/supervise` skill: the fleet-supervisor's loop, shipped with the app and
 * installed (idempotently) as a personal skill the moment a supervisor session
 * is spawned, into the directory ITS harness reads (`~/.claude/skills` for
 * Claude, `$CODEX_HOME/skills` for Codex — same SKILL.md format). Keeping it a
 * real skill — not just system-prompt text — means the user can read and edit it, and
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
import { agentSkillDir } from '../lib/agentSkills';
import type { AgentProvider } from './agentProviders';

const SKILL_NAME = 'supervise';

/** The skill text. Parameterized ONLY on its own install directory: the helper
 *  script is invoked by absolute path, and that path differs per harness
 *  (`~/.claude/skills/supervise` vs `$CODEX_HOME/skills/supervise`). Everything
 *  else is identical across providers on purpose — one doctrine, no fork. */
const skillBody = (dir: string): string => `---
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
node "${dir}/fleet.mjs" status [--active]
node "${dir}/fleet.mjs" convo <sessionId> --since <seq>
node "${dir}/fleet.mjs" reply <sessionId>
\`\`\`

- \`status\` — one line per session: id, mode, what it's blocked on (and for how
  long), context use, label, cwd. Rows are sorted blocked-first, then the other
  live sessions, then stopped ones. \`--active\` drops the stopped sessions.
  - \`blocked=approval:Bash (45s)\` — the age comes from the conversation item the
    approval is parked on; a \`~\` prefix (\`(~2m)\`) means it was estimated from the
    session's last activity instead.
  - \`label=\` is the daemon's label when it has one. It doesn't today — labels
    live in the desktop store, so \`list_agents\` / \`get_snapshot\` are the only
    authoritative source — so the script derives one: an agent worktree renders
    as \`repo: the spawn label\`, anything else as the cwd basename.
- \`convo <id> --since <seq>\` — prints \`seq=<latest>\` then only the turns after
  \`<seq>\`, condensed (the API returns each tool call/result twice — a streamed
  and a final copy — and the script dedups them). Omit \`--since\` for the whole
  conversation.
- \`reply <id>\` — just that session's latest assistant message (how you read a
  worker's digest).

Use the \`mcp__workspacer__*\` tools for ACTIONS: spawn_agent, send_message,
notify, approve, answer. (\`list_agents\` / \`get_conversation\` exist too, but
prefer the script for routine reads.)

The script talks to THIS machine's claudemon only. The fleet may span machines
(federated hubs): sessions on a peer hub never appear in \`fleet.mjs\` output, so
use \`list_agents\` for fleet discovery — a row with a \`hub\` field lives on that
peer, and you must pass that hub value through to the per-session tools
(get_conversation, get_snapshot, send_message, approve, …) when reading or
acting on it.

## 0. Settings

Call \`get_config\` once and read \`supervisor.summarizerModel\` (fallback
\`sonnet\`), \`supervisor.pollSeconds\` (fallback \`45\`) and
\`supervisor.fullAccess\` (fallback \`false\`). Your system prompt also states
these as a fallback if the config call fails.

## 1. Keep one cheap summarizer worker

Spawn a single long-lived digest worker the first time through, then reuse it:

\`\`\`
spawn_agent({
  model: <summarizerModel>,
  toolScope: "view",        // read-only workspacer tools: transcripts, snapshots
  label: "fleet digest",
  parentSessionId: <your session id>,
  cwd: <any active agent's cwd, or the host cwd>
})
\`\`\`

\`toolScope: "view"\` gives the worker the read-only workspacer tools (so it can
call get_transcript / get_conversation itself) and nothing else — it cannot
spawn, message, or approve, and it pays context for only the handful of
read-tool schemas. It does NOT make it a supervisor: it just reads transcripts
and answers you. Reusing one worker keeps cost down; only spawn another if the
first dies. (\`mcpFacade: true\` is the legacy spelling and grants the FULL tool
set — prefer the scoped form.)

When \`supervisor.fullAccess\` is true you run with permissions bypassed, and
every worker you spawn should inherit that: also pass \`skipPermissions: true\`
to \`spawn_agent\` so workers never stall on an approval prompt. (Your session
token carries the grant that makes the request honored; when the setting is
off, don't pass it — the request would be clamped anyway.)

## 2. Each pass — work incrementally

Keep a per-agent cursor: the last conversation \`seq\` you have digested.

1. Run \`fleet.mjs status\` to see the fleet and who is blocked. Ignore your own
   session and the digest worker. If federated peer hubs are linked, also check
   \`list_agents\` for remote sessions (rows with a \`hub\` field) the script
   cannot see.
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
//   node fleet.mjs status [--active]        fleet overview (label, mode, blocked, ctx, cwd)
//   node fleet.mjs convo <id> [--since N]   latest seq + only the turns after N
//   node fleet.mjs reply <id>               that session's latest assistant message
//
// status rows are sorted blocked-first, then other live sessions, then stopped
// ones (most recently active first inside each group). --active drops the
// stopped sessions entirely.
//
// Notes on the data, learned by inspecting the live API:
//  * The daemon carries NO human label — /sessions rows have no label/name/title
//    field (the desktop app keeps labels in its own store, which is what
//    mcp__workspacer__list_agents / get_snapshot return). So label= prefers a
//    real field if one ever appears and otherwise DERIVES one from the cwd: an
//    agent worktree (~/.workspacer/worktrees/<repo>/<slug>) carries the spawn
//    label slugified into <slug>, so it renders as "repo: the slug words";
//    anything else falls back to the cwd basename.
//  * The conversation endpoint returns most items TWICE — the live streamed copy
//    and the transcript-derived one, identical apart from timestamp precision
//    (tool calls, tool results, and assistant/user text all twin). convo dedups
//    on tool id (and, for text, on the text plus a same-second timestamp),
//    keeping the most complete copy in its original position.
//  * pending has no timestamp of its own, so blocked-age is read from the
//    timestamp of the conversation item the approval is waiting on (exact); if
//    that item can't be found the session's updated_at is used instead and the
//    age is prefixed with a tilde.
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

// --- conversation dedup ------------------------------------------------------

// Stable identity of a conversation item. Tool items carry an id; text items
// don't, so they are keyed by their text and only merged when the two copies
// are near-simultaneous (see areTwins) — a user who really does send "continue"
// twice must still see it twice.
function itemKey(it) {
  if (it.kind === 'tool_use' && it.id) return 'u:' + it.id;
  if (it.kind === 'tool_result' && it.tool_use_id) return 'r:' + it.tool_use_id;
  if (it.kind === 'user_message' || it.kind === 'assistant_text') return 't:' + it.kind + '|' + (it.text || '');
  return null;
}

// Two copies of the same item, or the same text sent twice? The twins land
// within a second of each other (the streamed copy has nanosecond precision,
// the final one milliseconds); a copy with no timestamp at all is always a twin.
const TWIN_WINDOW_MS = 5000;
function areTwins(a, b) {
  const ta = Date.parse(a.timestamp || '');
  const tb = Date.parse(b.timestamp || '');
  if (!Number.isFinite(ta) || !Number.isFinite(tb)) return true;
  return Math.abs(ta - tb) <= TWIN_WINDOW_MS;
}

// How complete a copy is: number of populated fields first, total content
// length as the tie-break. Twins usually differ only in timestamp precision,
// but this survives a genuinely truncated streaming copy too.
function completeness(it) {
  let fields = 0;
  let len = 0;
  for (const k of Object.keys(it)) {
    const v = it[k];
    if (v == null || v === '') continue;
    fields++;
    len += (typeof v === 'string' ? v : JSON.stringify(v)).length;
  }
  return fields * 1e9 + len;
}

function dedup(items) {
  const out = [];
  const seen = new Map(); // key -> index of the last kept copy in out
  for (const it of items) {
    const k = itemKey(it);
    const i = k == null ? undefined : seen.get(k);
    const isTwin = i != null && (k[0] !== 't' || areTwins(out[i], it));
    if (isTwin) {
      if (completeness(it) > completeness(out[i])) out[i] = it; // keep position, upgrade copy
      continue;
    }
    if (k != null) seen.set(k, out.length);
    out.push(it);
  }
  return out;
}

// --- status ------------------------------------------------------------------

function ago(sec) {
  if (!(sec >= 0)) return null;
  if (sec < 60) return Math.round(sec) + 's';
  const m = Math.floor(sec / 60);
  const s = Math.round(sec % 60);
  if (m < 60) return m + 'm' + (s ? s + 's' : '');
  const h = Math.floor(m / 60);
  return h + 'h' + (m % 60) + 'm';
}

function secsSince(ts) {
  if (!ts) return null;
  const t = Date.parse(ts);
  if (!Number.isFinite(t)) return null;
  return Math.max(0, (Date.now() - t) / 1000);
}

// The tool call an approval/question is parked on. Its conversation item's
// timestamp is when the session actually blocked.
function pendingToolUseId(p) {
  const raw = (p && p.raw) || {};
  return raw.tool_use_id || raw.toolUseId || null;
}

// Exact blocked-age: fetch just the tail of the conversation (a since= beyond
// the latest seq returns the seq alone, ~90 bytes) and read the timestamp of
// the item the approval is waiting on. Null when it can't be found.
async function pendingAgeSecs(id, pending) {
  const tuid = pendingToolUseId(pending);
  if (!tuid) return null;
  const path = '/sessions/' + encodeURIComponent(id) + '/conversation';
  try {
    const head = await getJSON(path + '?since=' + Number.MAX_SAFE_INTEGER);
    const seq = Number(head && head.seq) || 0;
    if (!seq) return null;
    const tail = await getJSON(path + '?since=' + Math.max(0, seq - 40));
    const items = Array.isArray(tail.items) ? tail.items : [];
    for (let i = items.length - 1; i >= 0; i--) {
      const it = items[i];
      if ((it.id === tuid || it.tool_use_id === tuid) && it.timestamp) return secsSince(it.timestamp);
    }
  } catch { /* fall back to updated_at */ }
  return null;
}

function blockedOf(p) {
  if (!p) return '-';
  if (p.kind === 'approval') return 'approval:' + (p.tool || '?');
  if (p.kind === 'question') return 'question';
  return p.kind || '?';
}

// The daemon exposes no label; prefer one if it ever does, else derive from cwd.
function labelOf(s) {
  for (const k of ['label', 'name', 'title']) {
    if (typeof s[k] === 'string' && s[k].trim()) return trunc(s[k], 48);
  }
  const cwd = s.cwd || '';
  const wt = cwd.match(/[.]workspacer[/]worktrees[/]([^/]+)[/]([^/]+)[/]?$/);
  if (wt) {
    const repo = wt[1];
    const slug = wt[2].startsWith(repo + '-') ? wt[2].slice(repo.length + 1) : wt[2];
    return trunc(repo + ': ' + slug.replace(/[-_]+/g, ' '), 48);
  }
  const base = cwd.replace(/[/]+$/, '').split('/').pop();
  return base ? trunc(base, 48) : '-';
}

function ctxOf(s) {
  const sl = s.status_line || {};
  if (sl.context_used_pct != null) return Math.round(sl.context_used_pct) + '%';
  const u = s.usage || {};
  const tok = u.context_tokens != null ? u.context_tokens : u.contextTokens;
  if (!tok) return '-';
  const limit = u.context_limit != null ? u.context_limit : u.contextLimit;
  if (limit) return Math.round((tok / limit) * 100) + '%';
  return tok + 'tok';
}

// blocked first, then everything else still alive, stopped last; most recently
// active first inside each group.
function rankOf(s) {
  if (s.pending) return 0;
  return s.mode === 'stopped' ? 2 : 1;
}

async function status(activeOnly) {
  let sessions = await getJSON('/sessions');
  if (!Array.isArray(sessions) || sessions.length === 0) { console.log('(no sessions)'); return; }
  if (activeOnly) sessions = sessions.filter((s) => s.mode !== 'stopped');
  if (sessions.length === 0) { console.log('(no active sessions)'); return; }
  sessions.sort((a, b) => rankOf(a) - rankOf(b) || String(b.updated_at || '').localeCompare(String(a.updated_at || '')));

  // Age is only fetched for the (few) blocked sessions — the common case stays
  // a single request.
  const ages = new Map();
  await Promise.all(
    sessions.filter((s) => s.pending).map(async (s) => {
      let secs = await pendingAgeSecs(s.session_id, s.pending);
      let exact = secs != null;
      if (secs == null) secs = secsSince(s.updated_at);
      const txt = ago(secs);
      if (txt) ages.set(s.session_id, ' (' + (exact ? '' : '~') + txt + ')');
    }),
  );

  for (const s of sessions) {
    console.log(
      'session:' + s.session_id +
      '  mode=' + s.mode +
      '  blocked=' + blockedOf(s.pending) + (ages.get(s.session_id) || '') +
      '  ctx=' + ctxOf(s) +
      '  label=' + labelOf(s) +
      '  cwd=' + (s.cwd || '-'),
    );
  }
}

async function convo(id, since) {
  if (!id) throw new Error('convo needs <id>');
  const q = since != null ? ('?since=' + encodeURIComponent(since)) : '';
  const data = await getJSON('/sessions/' + encodeURIComponent(id) + '/conversation' + q);
  console.log('seq=' + (data.seq || 0));
  const items = dedup(Array.isArray(data.items) ? data.items : []);
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
const activeOnly = argv.includes('--active');

(async () => {
  try {
    if (cmd === 'status') await status(activeOnly);
    else if (cmd === 'convo') await convo(argv[1], sinceVal);
    else if (cmd === 'reply') await reply(argv[1]);
    else { console.error('usage: fleet.mjs status [--active] | convo <id> [--since N] | reply <id>'); process.exit(2); }
  } catch (e) { console.error(String((e && e.message) || e)); process.exit(1); }
})();
`;

/** Directory the skill (and its helpers) are installed into for `provider`, or
 *  null when that harness has no personal-skills convention (lib/agentSkills). */
function skillDir(provider: AgentProvider): string | null {
  return agentSkillDir(provider, SKILL_NAME);
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
  try {
    current = fs.readFileSync(file, 'utf8');
  } catch {
    /* not installed yet */
  }
  if (current !== content) fs.writeFileSync(file, content, 'utf8');
}

/**
 * Install the `/supervise` skill (SKILL.md + the fleet.mjs parsing helper) into
 * the user's personal Claude Code skills dir, refreshing them so the supervisor
 * always runs the current version. Best-effort: a failure just means the
 * supervisor falls back to its system prompt / the MCP tools. Safe to call on
 * every supervisor spawn.
 */
export function installSupervisorSkill(provider: AgentProvider = 'claude'): void {
  const dir = skillDir(provider);
  if (!dir) {
    console.warn(
      `[supervisorSkill] ${provider} has no known personal-skills directory — skipping /supervise`,
    );
    return;
  }
  try {
    fs.mkdirSync(dir, { recursive: true });
    writeIfChanged(path.join(dir, 'SKILL.md'), skillBody(dir));
    writeIfChanged(path.join(dir, 'fleet.mjs'), FLEET_SCRIPT);
  } catch {
    /* installing the skill is best-effort */
  }
}
