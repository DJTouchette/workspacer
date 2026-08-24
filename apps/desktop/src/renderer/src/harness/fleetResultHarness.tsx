/**
 * Standalone fleet-wake harness — the structured-result card as a manager
 * meets it, with no Electron, no hub and no live fleet. Every wake below is
 * built by the REAL builder (main/shared/fleetMessages) and rendered by the
 * real ConversationMessage, so what you see is what a manager's transcript
 * shows.
 *
 * The payloads are real results from dispatches on 2026-08-23 plus the shapes
 * that break cards: a schema nobody anticipated, a required field the worker
 * never sent, an empty array, a 30-item list, and a result too large for the
 * wake (truncated, and so unparseable).
 *
 * Open http://localhost:5173/fleet-result-harness.html with the dev server
 * running. `?theme=<id>` switches themes; the chat column narrows with the
 * window, which is how to check the narrow-pane case.
 */
import React from 'react';
import { createRoot } from 'react-dom/client';
import '../App.css';
import { ConversationMessage } from '../components/claude/ConversationMessage';
import { buildFleetMessage, type FleetMessageEntry } from '../../../main/shared/fleetMessages';
import { applyTheme, resolveTheme } from '../themes';

const params = new URLSearchParams(location.search);
applyTheme(resolveTheme(params.get('theme') ?? 'everforest'));

const json = (o: unknown): string => JSON.stringify(o, null, 2);

/** The mobile-e2e repair, exactly as that worker reported it. */
const MOBILE_E2E = {
  commit: 'e124a078',
  merged: true,
  testsFixed: 8,
  filesChanged: [
    'apps/desktop/tests/e2e/mobileClient.test.ts',
    'services/hub/cmd/hub/mobile.html',
    '.github/workflows/ci.yml',
  ],
  realBugsFound: [
    'mobile.html fetchConv(): the conversation-poll self-rearm only fires via a full render() call, but a no-op fetch (empty/turnless conversation) deliberately skips render() to avoid a past DOM-thrash bug — which also silently stopped all future polling after the first fetch. An agent with no conversation yet would never get its transcript checked again once the chat screen opened.',
  ],
  checksRun: [
    'npx playwright test tests/e2e/mobileClient.test.ts (run twice for stability, 22/22 both times)',
    'npx vitest run (apps/desktop main): 2154/2154, matches baseline',
    'npx vitest run (apps/desktop/src/renderer): 1142/1142, matches baseline',
    'npm run typecheck (main + renderer tsc --noEmit): clean',
    'npm run format:check (prettier, desktop tree): clean',
  ],
  followUps: [
    'chatTailPin.test.ts and libraryPane.test.ts remain unwired from CI; libraryPane needs a built Electron app — sizeable separate task, not done here',
    "Watch the first live CI run of the new 'Mobile client e2e (Playwright)' step in ci.yml",
  ],
  caveats:
    'ci.yml change was not run through actionlint or an actual GHA runner (no such tool available in this environment) — it mirrors the existing hub job exactly and was sanity-checked for indentation/structure only. Did not push per instructions; master is still ~21 commits ahead of origin as before.',
};

/** A schema this card has never seen: every key invented per dispatch. */
const UNANTICIPATED = {
  decisionTaken: 'kept the PTY transport as the default and gated stream mode behind config',
  bytesAdded: 4211,
  secretsCheck: { scanned: 41, findings: 'none', tool: 'gitleaks 8.18' },
  itemsSkipped: [],
  rounds: [
    { lens: 'correctness', verdict: 'confirmed' },
    { lens: 'security', verdict: 'refuted' },
  ],
  caveats: '',
};

const CASES: Array<{ title: string; note: string; entry: FleetMessageEntry }> = [
  {
    title: 'A real result',
    note: 'The mobile-e2e repair: a boolean, a count, a paragraph in an unknown array, five checks, two follow-ups, a two-sentence caveat.',
    entry: {
      label: 'mobile e2e: repair the suite',
      sessionId: 'b709c31b',
      cwd: '/home/u/Work/workspacer',
      lastReply:
        'Repaired all 8 red mobileClient.test.ts tests, found and fixed a real conversation-poll bug, wired the suite into CI. Merged to master.',
      result: json(MOBILE_E2E),
    },
  },
  {
    title: 'A schema nobody anticipated',
    note: 'Not one conventional key except caveats — which the worker answered as empty, and which reads as an answer rather than as silence.',
    entry: {
      label: 'transport: decide the default',
      sessionId: 'c2f10a44',
      cwd: '/home/u/Work/workspacer',
      lastReply: 'Went with PTY. Details in the result.',
      result: json(UNANTICIPATED),
    },
  },
  {
    title: 'A partial result',
    note: 'The dispatch required commit and filesChanged; this worker ran out of context and sent neither. The card shows the truth it got.',
    entry: {
      label: 'docs: rewrite the landing copy',
      sessionId: 'd41d8cd9',
      cwd: '/home/u/Work/workspacer/landing',
      lastReply: 'I did not finish — I ran out of context mid-rewrite.',
      result: json({
        merged: false,
        caveats:
          'I ran out of context before the second half of the page. Nothing was committed, so nothing is at risk, but the branch has uncommitted edits in landing/index.html.',
      }),
    },
  },
  {
    title: 'A long list',
    note: 'Thirty checks. The head shows; the rest is one click away.',
    entry: {
      label: 'audit: sweep every plugin manifest',
      sessionId: 'a1b2c3d4',
      cwd: '/home/u/Work/workspacer-plugins',
      result: json({
        merged: true,
        commit: '9ae2586612ab4c0d99e1f77aa0b3c4d5e6f70819',
        pluginsChecked: 30,
        checksRun: Array.from(
          { length: 30 },
          (_, i) => `plugin ${i + 1}: manifest version present, tools declared, sandbox scoped`,
        ),
        caveats: 'Three catalog repos were unreachable and were skipped rather than guessed at.',
      }),
    },
  },
  {
    title: 'A result too large for the wake',
    note: 'The wake caps a result at 8KB and says so — which leaves the JSON truncated mid-string. The bytes that arrived still show.',
    entry: {
      label: 'migrate: 400 call sites',
      sessionId: 'e5f6a7b8',
      cwd: '/home/u/Work/workspacer',
      result:
        '{\n  "commit": "600a2c4e",\n  "merged": true,\n  "filesChanged": [\n    "apps/desktop/src/main/services/claudeSpawn.ts",\n    "apps/desktop/src/main/services/managedSpawn.ts",\n    "apps/desktop/src/renderer/src/components/claude/Compo\n[truncated: 8192 bytes of validated result]',
    },
  },
  {
    title: 'No result at all',
    note: 'The dispatch asked for a schema and the worker ignored it. The card says so instead of showing nothing; the prose report is still under "last reply".',
    entry: {
      label: 'rust: stream approvals',
      sessionId: 'f9e8d7c6',
      cwd: '/home/u/Work/workspacer/services/claudemon',
      lastReply:
        'Approvals now round-trip through the control protocol. I did not run the Rust suite — cargo was mid-build.',
      resultError:
        "no `wks-result` block in the worker's final message — it did not honor the result contract",
    },
  },
];

function Harness(): React.ReactElement {
  return (
    <div
      style={{
        background: 'var(--wks-claude-bg)',
        minHeight: '100vh',
        padding: '24px 16px 60px 16px',
        fontFamily: 'var(--wks-font-sans)',
      }}
    >
      <div style={{ maxWidth: 'var(--wks-chat-width)', margin: '0 auto' }}>
        {CASES.map((c) => (
          <section key={c.title} style={{ marginBottom: 28 }}>
            <div
              style={{
                fontSize: '0.6rem',
                fontWeight: 700,
                letterSpacing: '0.08em',
                textTransform: 'uppercase',
                color: 'var(--wks-text-faint)',
              }}
            >
              {c.title}
            </div>
            <div
              style={{
                margin: '2px 0 8px 0',
                fontSize: '0.72rem',
                color: 'var(--wks-text-tertiary)',
              }}
            >
              {c.note}
            </div>
            <ConversationMessage
              turn={{
                role: 'user',
                content: buildFleetMessage('worker-finished', [c.entry]),
                timestamp: 0,
              }}
            />
          </section>
        ))}
      </div>
    </div>
  );
}

createRoot(document.getElementById('root')!).render(<Harness />);
