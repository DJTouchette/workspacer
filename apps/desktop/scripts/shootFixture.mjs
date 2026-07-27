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
 *      assistant/user message text AND of the GUI work log — assistant rows
 *      carry `tool_use` blocks, user rows carry the joined `tool_result`s
 *      (`conversation.rs::items_from_row`). Hooks never carry any of it.
 *   2. `POST :7890/hook/session_start` with `transcript_path` binds the session
 *      to that file. `read_at()` treats the hook's path as authoritative, so no
 *      cwd guessing is involved.
 *   3. `POST :7890/statusline` attaches model, context %, cost, and rate-limit
 *      windows — the only channel that carries them. The payload must use
 *      Claude's real statusLine keys (`context_window.used_percentage`,
 *      `cost.total_cost_usd`, `rate_limits.five_hour` — see
 *      `state.rs::StatusLine::from_claude_json`); invented keys parse to None.
 *   4. Pending states use the hook that actually drives each card:
 *      - approval: `POST /hook/permission` (PermissionRequest). For Claude PTY
 *        sessions the desktop ignores the daemon's parked `pending` slot
 *        entirely (claudeSessionStore.ts `daemonOwnsPending`) — only a
 *        PermissionRequest hook sets `pendingApproval` (hookEventRouter.ts).
 *        A gated `pre_tool` would also hold the HTTP response open for 30s.
 *      - question: `POST /hook/pre_tool` with tool_name `AskUserQuestion` and
 *        `tool_input.questions` — surfaced as a pending picker on both the
 *        daemon (state.rs) and desktop (hookEventRouter.ts) paths, no gate.
 *      - working: an ordinary `pre_tool` parks an active tool call and flips
 *        the session to streaming.
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

/** Real Claude transcript shapes (`transcript.rs::blocks`): `type` is the row
 *  role, content blocks hang off `message.content`. */
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

// The hero conversation: a finished prep pass with a real work log — text
// introducing collapsed tool steps, then edits that feed the end-of-turn
// changed-files card (closed by the follow-up user turn, so the card renders
// even while a question is pending later).
const heroLines = [
  userLine('final prep pass before we flip public', iso(11 * min)),
  asstLine(
    [
      text('Running the secret scanner over the full history first.'),
      toolUse('tu_scan', 'Bash', {
        command: 'gitleaks detect --source . --log-opts="--all"',
        description: 'Scan full git history for secrets',
      }),
    ],
    iso(10 * min),
  ),
  resultLine('tu_scan', 'no leaks found — 1,284 commits scanned in 41s', iso(10 * min)),
  asstLine(
    [
      toolUse('tu_ci', 'Read', {
        file_path: '/home/dev/work/workspacer/.github/workflows/release.yml',
      }),
    ],
    iso(9 * min),
  ),
  resultLine('tu_ci', '82 lines', iso(9 * min)),
  asstLine(
    [
      text(
        'History is clean and CI already builds installers for all three OSes. Wiring the landing download buttons to the draft release now.',
      ),
      toolUse('tu_dl', 'Edit', {
        file_path: '/home/dev/work/workspacer/landing/index.html',
        old_string:
          '<a class="dl" href="#" data-os="mac" aria-disabled="true">\n  Download for macOS (soon)\n</a>',
        new_string:
          '<a class="dl" href="https://github.com/worky/workspacer/releases/latest/download/Workspacer.dmg" data-os="mac">\n  Download for macOS\n</a>\n<span class="dl-meta">v0.124.0 · universal · 142 MB</span>',
      }),
    ],
    iso(7 * min),
  ),
  resultLine('tu_dl', 'The file landing/index.html has been updated.', iso(7 * min)),
  asstLine(
    [
      toolUse('tu_docs', 'Edit', {
        file_path: '/home/dev/work/workspacer/landing/docs.html',
        old_string: '<!-- downloads table placeholder -->',
        new_string:
          '<table class="downloads">\n  <tr><td>macOS</td><td>Workspacer.dmg</td></tr>\n  <tr><td>Linux</td><td>Workspacer.AppImage</td></tr>\n  <tr><td>Windows</td><td>Workspacer-Setup.exe</td></tr>\n</table>',
      }),
    ],
    iso(6 * min),
  ),
  resultLine('tu_docs', 'The file landing/docs.html has been updated.', iso(6 * min)),
  userLine('anything left before I hit the button?', iso(5 * min)),
  asstLine(
    [
      text(
        'All three prep items are done — history is clean, downloads are wired, and the landing page is ready to flip public.',
      ),
    ],
    iso(4 * min),
  ),
];

const AGENTS = [
  {
    id: 'stage-workspacer',
    cwd: '/home/dev/work/workspacer',
    lines: heroLines,
    // Its turn is over in the transcript, so close it after the edit hooks.
    settled: true,
    // Mirrors the two Edits in `heroLines` — same ids, so the work log and the
    // Inspector's Files tab agree instead of contradicting each other.
    edits: [
      {
        id: 'tu_dl',
        tool: 'Edit',
        input: { file_path: '/home/dev/work/workspacer/landing/index.html' },
      },
      {
        id: 'tu_docs',
        tool: 'Edit',
        input: { file_path: '/home/dev/work/workspacer/landing/docs.html' },
      },
    ],
    statusline: {
      model: { display_name: 'Fable 5' },
      context_window: {
        used_percentage: 32,
        total_input_tokens: 96000,
        total_output_tokens: 145000,
      },
      cost: { total_cost_usd: 46.24 },
      rate_limits: {
        five_hour: { used_percentage: 38, resets_at: Math.floor(Date.now() / 1000) + 9400 },
        seven_day: { used_percentage: 61, resets_at: Math.floor(Date.now() / 1000) + 275000 },
      },
    },
  },
  {
    id: 'stage-prep',
    cwd: '/home/dev/work/prep',
    lines: [
      userLine('where is the release pipeline at?', iso(5 * min)),
      asstLine([text('Checking the release pipeline before I summarize.')], iso(3 * min)),
    ],
    statusline: {
      model: { display_name: 'Opus 4.8' },
      context_window: { used_percentage: 17, total_input_tokens: 33000, total_output_tokens: 4200 },
      cost: { total_cost_usd: 0.21 },
    },
    // A live tool call: parks an active Bash card and reads as "working".
    working: {
      tool_name: 'Bash',
      tool_use_id: 'tu_watch',
      tool_input: { command: 'gh run watch 8123456789', description: 'Watch the release build' },
    },
  },
  {
    id: 'stage-rivet',
    cwd: '/home/dev/work/rivet',
    lines: [
      userLine('verify the coverage numbers', iso(7 * min)),
      asstLine(
        [text('I need to run the coverage query against the prod snapshot to be sure.')],
        iso(4 * min),
      ),
    ],
    statusline: {
      model: { display_name: 'Opus 4.8' },
      context_window: { used_percentage: 8, total_input_tokens: 12000, total_output_tokens: 1800 },
      cost: { total_cost_usd: 0.34 },
    },
    // PermissionRequest → pendingApproval → the NEED YOU card.
    approval: {
      tool_name: 'Bash',
      tool_input: { command: 'psql prod -c "select count(*) from coverage"' },
    },
  },
  {
    id: 'stage-recon',
    cwd: '/home/dev/work/recon',
    lines: [
      userLine('map the cache layout', iso(2 * min)),
      asstLine(
        [text('Mapping the cache layout — three tiers, symbol index is the hot one.')],
        iso(40000),
      ),
    ],
    statusline: {
      model: { display_name: 'Opus 4.8' },
      context_window: { used_percentage: 3, total_input_tokens: 5200, total_output_tokens: 900 },
      cost: { total_cost_usd: 0.05 },
    },
  },
];

export const HERO = { id: 'stage-workspacer', cwd: '/home/dev/work/workspacer' };

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

    // The Inspector's Files tab and the fleet file stats read `fileChanges`,
    // which for Claude sessions is built from PreToolUse hooks only — see
    // `conversationApplier.ts::recordManagedFileChange`. Edits that exist just
    // in the transcript render in the work log but leave the rail reading
    // "No files changed yet" beside a CHANGED FILES (2) card, which looks
    // broken. Each pre_tool is paired with a post_tool so the call closes and
    // the session settles back to idle instead of reading as "working".
    for (const e of a.edits ?? []) {
      const base = { session_id: a.id, cwd: a.cwd, tool_name: e.tool, tool_use_id: e.id };
      await post(`${HOOK}/hook/pre_tool`, { ...base, tool_input: e.input });
      await post(`${HOOK}/hook/post_tool`, {
        ...base,
        tool_input: e.input,
        tool_response: { success: true },
      });
    }

    // A trailing post_tool leaves the session mid-turn: the status bar reads
    // "Streaming" and a thinking indicator sits under the last message, which
    // is wrong for a turn the transcript shows as finished. Stop closes it.
    if (a.settled) {
      await post(`${HOOK}/hook/stop`, { session_id: a.id, cwd: a.cwd });
    }

    if (a.working) {
      await post(`${HOOK}/hook/pre_tool`, { session_id: a.id, cwd: a.cwd, ...a.working });
    }
    if (a.approval) {
      await post(`${HOOK}/hook/permission`, { session_id: a.id, cwd: a.cwd, ...a.approval });
    }
  }
  return AGENTS.map((a) => a.id);
}

/**
 * Put the hero into the pending-question state — fired AFTER the work-log
 * capture, since it flips ambient to waiting_input (which would suppress a
 * trailing changed-files card; the hero's edits are closed by a later user
 * turn, so its card survives).
 * Mirrors the 4-option fixture in `harness/deckHarness.tsx`.
 */
export async function fireQuestion() {
  await post(`${HOOK}/hook/pre_tool`, {
    session_id: HERO.id,
    cwd: HERO.cwd,
    tool_name: 'AskUserQuestion',
    tool_use_id: 'tu_ask',
    tool_input: {
      questions: [
        {
          question: 'Workspacer is ready to go public — what should we do next?',
          header: 'Next move',
          multiSelect: false,
          options: [
            {
              label: 'Tag v0.124.0 and ship it (Recommended)',
              description:
                'Push the commits, tag a release so CI builds installers for all three OSes, and publish the draft so the landing page download button goes live.',
            },
            {
              label: 'One more design pass',
              description: 'Give another pane the Inspector treatment before the public debut.',
            },
            {
              label: 'Write the announcement',
              description:
                'Draft the launch post: control plane for a fleet of coding agents, alpha, source-available.',
            },
            {
              label: 'Just keep hacking',
              description: 'Public can wait — spawn an agent and build the next feature.',
            },
          ],
        },
      ],
    },
  });
}
