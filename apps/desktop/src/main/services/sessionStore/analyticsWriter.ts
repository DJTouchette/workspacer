import * as fs from 'fs';
import * as path from 'path';
import { sessionHistory } from '../sessionHistory';
import type { ClaudeSessionState } from '../claudeSessionStore';

// ── SessionAnalyticsWriter ────────────────────────────────────────────────────

/** Best-effort current git branch for a working dir (reads .git/HEAD). */
export function gitBranchOf(cwd: string): string {
  if (!cwd) return '';
  try {
    const head = fs.readFileSync(path.join(cwd, '.git', 'HEAD'), 'utf-8').trim();
    const m = head.match(/^ref:\s*refs\/heads\/(.+)$/);
    return m ? m[1] : head.slice(0, 12); // detached HEAD → short sha
  } catch {
    return '';
  }
}

/** Snapshot this session's metadata into the analytics history store. */
export function writeHistory(session: ClaudeSessionState, status: 'active' | 'ended'): void {
  const now = Date.now();
  const workflowFailed = session.workflows.filter((w) => w.status === 'failed').length;
  const subagentCount =
    session.subagents.length + session.workflows.reduce((n, w) => n + w.agents.length, 0);
  sessionHistory.record({
    sessionId: session.sessionId,
    cwd: session.cwd,
    agentName: session.cwd
      ? path.basename(session.cwd.replace(/[/\\]+$/, ''))
      : session.sessionId.slice(0, 8),
    provider: session.provider ?? '',
    model: session.usage?.model ?? '',
    gitBranch: gitBranchOf(session.cwd),
    startedAt: new Date(session.startedAt).toISOString(),
    endedAt: status === 'ended' ? new Date(now).toISOString() : '',
    durationMs: now - session.startedAt,
    inputTokens: session.usage?.totalInputTokens ?? 0,
    outputTokens: session.usage?.totalOutputTokens ?? 0,
    costUSD: session.usage?.costUSD ?? 0,
    peakContext: session.peakContext,
    toolCalls: session.totalToolCalls,
    messageCount: session.conversation.length,
    subagentCount,
    workflowRuns: session.workflows.length,
    workflowFailed,
    status,
  });
  // Per-model split (main thread + subagent turns). Cumulative totals, so the
  // replace-style upsert stays idempotent across repeated snapshots.
  if (session.usage?.models) sessionHistory.recordModels(session.sessionId, session.usage.models);
}
