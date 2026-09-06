/** Production Fleet component with an explicitly mocked evidence transport. */
import React from 'react';
import { createRoot } from 'react-dom/client';
import '../App.css';
import { ConversationMessage } from '../components/claude/ConversationMessage';
import { HtmlCardHostProvider } from '../components/claude/HtmlResponseCard';
import { buildFleetMessage } from '../../../main/shared/fleetMessages';
import type { FleetReviewEvidence, FleetReviewRequest } from '../../../main/shared/fleetReview';
import { REVIEW_REQUEST_FILE_EVENT } from '../lib/reviewBus';
import { SESSION_WATCH_EVENT, AGENT_WATCH_EVENT } from '../lib/watchBus';
import { applyTheme, resolveTheme } from '../themes';
const params = new URLSearchParams(location.search);
applyTheme(resolveTheme(params.get('theme') ?? 'everforest'));
const id = '11111111-1111-4111-8111-111111111111';
const evidence: FleetReviewEvidence = {
  id,
  ownerSessionId: 'manager',
  workerSessionId: 'worker',
  projectRoot: '/project/alpha',
  allocatedCwd: '/trees/worker',
  branch: 'wks/worker',
  baseCommit: 'a'.repeat(40),
  headCommit: 'b'.repeat(40),
  capturedAt: '2026-09-06T00:00:00Z',
  lifecycle: 'turn-ended',
  availability: 'captured',
  files: [{ path: 'src/changed.ts', status: 'M' }],
};
const calls: FleetReviewRequest[] = [];
let revoked = false;
const paneRequests: string[] = [];
for (const event of [REVIEW_REQUEST_FILE_EVENT, SESSION_WATCH_EVENT, AGENT_WATCH_EVENT])
  window.addEventListener(event, () => paneRequests.push(event));
Object.assign(window, { reviewCalls: calls, paneRequests });
window.electronAPI = {
  fleetReviewRead: async (request: FleetReviewRequest) => {
    calls.push(request);
    if (revoked || request.ownerSessionId !== 'manager' || request.evidenceId !== id)
      return { ok: false, error: 'Review revoked or outside owner' };
    return {
      ok: true,
      evidence: {
        ...evidence,
        files: evidence.files.map((file) => ({
          ...file,
          diff: request.file
            ? 'diff --git a/src/changed.ts b/src/changed.ts\n--- a/src/changed.ts\n+++ b/src/changed.ts\n@@ -1 +1 @@\n-export const value = "base";\n+export const value = "captured change";\n'
            : undefined,
        })),
      },
    };
  },
  fleetReviewForget: async () => {
    revoked = true;
    return { ok: true };
  },
} as unknown as typeof window.electronAPI;
const message = buildFleetMessage('worker-finished', [
  {
    sessionId: 'worker',
    label: 'Alpha worker',
    cwd: '/trees/worker',
    reviewEvidenceId: id,
    result: JSON.stringify({
      commit: 'worker-reported-commit',
      checksRun: ['unit tests pass'],
      passed: true,
    }),
    lastReply: 'A complete worker report.',
  },
]);
createRoot(document.getElementById('root')!).render(
  <main
    style={{
      padding: 12,
      minHeight: '100vh',
      background: 'var(--wks-claude-bg)',
      color: 'var(--wks-text)',
      fontFamily: 'var(--wks-font-sans)',
    }}
  >
    <div style={{ maxWidth: 950, margin: '0 auto' }}>
      <HtmlCardHostProvider
        value={{ sessionId: 'manager', paneId: 'existing-pane', cwd: '/unrelated-manager' }}
      >
        <ConversationMessage turn={{ role: 'user', content: message, timestamp: 0 }} />
      </HtmlCardHostProvider>
    </div>
    <button>After review</button>
  </main>,
);
