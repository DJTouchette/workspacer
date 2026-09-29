/** Retained Electron history/schema/parser, without a private process or Electron ABI. */
import { afterAll, expect, it, vi } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { DatabaseSync } from 'node:sqlite';
import fixture from '../../../../../contracts/analytics-history-cases.json';
import { SessionHistoryStore, type SessionHistoryRecord } from './sessionHistoryCore';
import { MIGRATION_V2, MIGRATION_V3, MIGRATION_V4 } from './db/analyticsSchema';
import { recomputeSession, subagentFilesFor } from './analyticsUsage';

const root = vi.hoisted(
  () =>
    require('node:fs').mkdtempSync(
      require('node:path').join(require('node:os').tmpdir(), 'workspacer-analytics-contract-'),
    ) as string,
);
vi.mock('os', async (importOriginal) => ({
  ...(await importOriginal<typeof import('os')>()),
  homedir: () => root,
}));
afterAll(() => fs.rmSync(root, { recursive: true, force: true }));

it('persistent analytics corpus uses the retained Electron store and transcript parser', async () => {
  for (const [index, row] of fixture.cases.entries()) {
    const directory = path.join(root, String(index));
    fs.mkdirSync(directory, { recursive: true });
    const transcript = path.join(directory, 'claude-session.jsonl');
    const subagents = path.join(directory, 'claude-session/subagents');
    fs.mkdirSync(subagents, { recursive: true });
    fs.writeFileSync(transcript, row.mainJsonl);
    fs.writeFileSync(path.join(subagents, 'agent-sub.jsonl'), row.subagentJsonl);
    const usage = await recomputeSession(transcript, subagentFilesFor(transcript));
    expect(usage).not.toBeNull();
    const database = path.join(directory, 'history.sqlite');
    let db = new DatabaseSync(database);
    db.exec(MIGRATION_V2 + MIGRATION_V3 + MIGRATION_V4);
    const store = () =>
      new SessionHistoryStore(
        {
          db: {
            prepare(sql: string) {
              const statement = db.prepare(sql);
              statement.setAllowUnknownNamedParameters(true);
              return statement;
            },
          },
        } as unknown as ConstructorParameters<typeof SessionHistoryStore>[0],
        true,
      );
    let history = store();
    // These are the store's public DTO inputs, not a replacement aggregator.
    // Snapshot-to-record conversion remains exercised by the Rust corpus test.
    for (const snapshot of Object.values(row.snapshots)) {
      const managed =
        snapshot.session_id === 'codex-session'
          ? row.snapshots['codex-session'].status_line
          : undefined;
      const claude = snapshot.session_id === 'claude-session' ? usage! : undefined;
      const record: SessionHistoryRecord = {
        sessionId: snapshot.session_id,
        cwd: directory,
        provider: snapshot.provider,
        startedAt: snapshot.started_at,
        endedAt: '',
        status: 'ended',
        agentName: '',
        gitBranch: '',
        model: claude?.model ?? managed?.model_display ?? null,
        inputTokens: claude?.inputTokens ?? managed?.total_input_tokens ?? 0,
        outputTokens: claude?.outputTokens ?? managed?.total_output_tokens ?? 0,
        costUSD: claude?.costUSD ?? managed?.cost_usd ?? 0,
        peakContext: claude?.peakContext ?? 0,
        durationMs: 0,
        toolCalls: 0,
        messageCount: 0,
        subagentCount: 0,
        workflowRuns: 0,
        workflowFailed: 0,
      };
      history.record(record);
      history.record(record); // upsert must not double the session totals
      if (claude) history.recordModels(record.sessionId, claude.models);
    }
    const first = history.summary();
    expect(first.unavailable).toBeUndefined();
    expect(first.totals).toMatchObject({
      sessions: row.expected.sessions,
      unrecordedSessions: row.expected.unrecordedSessions,
      inputTokens: row.expected.inputTokens,
      outputTokens: row.expected.outputTokens,
    });
    expect(first.byModel).toHaveLength(row.expected.modelBuckets);
    expect(first.byModel.find((model) => model.key === 'gpt-5')?.costUSD).toBe(
      row.expected.managedCostUSD,
    );
    expect(history.summary('claude').totals.inputTokens).toBe(row.expected.claudeInputTokens);
    expect(history.summary('claude').byProvider).toHaveLength(row.expected.providerBuckets);
    expect(history.recent(1, 'codex')[0].sessionId).toBe('codex-session');
    db.close();
    fs.unlinkSync(transcript);
    db = new DatabaseSync(database);
    history = store();
    expect(history.summary().totals).toEqual(first.totals);
    expect(history.recent()).toHaveLength(row.expected.sessions);
    db.close();
  }
});
