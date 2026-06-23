/**
 * The `/supervise` skill: the fleet-supervisor's loop, shipped with the app and
 * installed (idempotently) as a personal Claude Code skill the moment a
 * supervisor session is spawned. Keeping it a real skill — not just system-prompt
 * text — means the user can read and edit it (`~/.claude/skills/supervise/SKILL.md`),
 * and the supervisor invokes it as `/supervise` and re-runs it on a loop.
 *
 * It is generic/static: per-session parameters (which summarizer model to spawn,
 * the loop cadence) are read at runtime from the workspacer config via the
 * `get_config` MCP tool, with the values also echoed into the supervisor's
 * system prompt as a fallback. Nothing here is assumed by the rest of the app —
 * the skill only does anything inside a supervisor session that has the
 * `mcp__workspacer__*` tools.
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
transcripts. You offload that reading to cheap summarizer workers.

If you do **not** have the \`mcp__workspacer__*\` tools, stop: this skill does
nothing outside a supervisor session.

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

1. \`list_agents\` to see the fleet (ids, state, model, context use, and any
   \`pendingApproval\` / \`pendingQuestions\`). Ignore your own session and the
   digest worker.
2. Decide who actually changed. For each agent, call
   \`get_conversation({ sessionId, sinceSeq: <last seq you saw> })\` — this returns
   ONLY the items after that seq, plus the latest \`seq\`. If \`items\` is empty,
   nothing changed: skip it. Otherwise it's cheap, and you can pass the new items
   straight to the worker (or, for a deeper read, have the worker pull the full
   transcript itself). Always advance your cursor to the returned \`seq\`.
3. Ask the digest worker to summarize only the changed agents (it does the
   reading, so this costs *its* context, not yours):
   \`\`\`
   send_message(<digestWorkerId>,
     "Here are the new turns for session <id> since I last looked: <the items>. " +
     "Update your running digest and reply with <=3 lines: GOAL / NOW / BLOCKED-ON (or 'not blocked').")
   \`\`\`
   Then poll \`get_snapshot(<digestWorkerId>)\` until its latest assistant turn
   holds the digest, and record it.
4. Maintain a short fleet status from those digests. When the user asks "what's
   everyone doing?", answer from this — don't re-read transcripts yourself.

## 3. Decisions — the important part

When an agent shows a \`pendingApproval\` or \`pendingQuestions\`, it is blocked on a
human. Assemble everything needed to decide and send ONE enriched notification:

- The approval's tool + input (e.g. the exact command or the diff), or the
  question + its options — these are already on \`list_agents\` / \`get_snapshot\`.
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

/** Absolute path to the installed skill file. */
function skillPath(): string {
  return path.join(os.homedir(), '.claude', 'skills', SKILL_NAME, 'SKILL.md');
}

/**
 * Write the `/supervise` skill to the user's personal Claude Code skills dir,
 * creating or refreshing it so the supervisor always runs the current version.
 * Best-effort: a failure here just means the supervisor falls back to its system
 * prompt. Safe to call on every supervisor spawn.
 */
export function installSupervisorSkill(): void {
  try {
    const file = skillPath();
    fs.mkdirSync(path.dirname(file), { recursive: true });
    // Only rewrite when the content changed, to avoid churning the user's file
    // (and any editor watching it) on every spawn.
    let current = '';
    try { current = fs.readFileSync(file, 'utf8'); } catch { /* not installed yet */ }
    if (current !== SKILL_BODY) fs.writeFileSync(file, SKILL_BODY, 'utf8');
  } catch {
    /* installing the skill is best-effort */
  }
}
