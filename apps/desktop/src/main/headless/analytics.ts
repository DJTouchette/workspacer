/** Persistent headless analytics using the native schema, queries and usage fold. */
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import type { DatabaseSync } from 'node:sqlite';
import type BetterSqlite3 from 'better-sqlite3';
import { getConfigDir } from '../services/configService';
import { claudeProfiles } from '../services/claudeProfiles';
import { assertPathAllowed } from '../lib/pathConfinement';
import { SessionHistoryStore, type SessionHistoryRecord } from '../services/sessionHistoryCore';
import { MIGRATION_V2, MIGRATION_V3, MIGRATION_V4 } from '../services/db/analyticsSchema';
import { recomputeSession, subagentFilesFor } from '../services/analyticsUsage';

let db: DatabaseSync;
let history: SessionHistoryStore;
let opening: Promise<void> | undefined;
let tail: Promise<unknown> = Promise.resolve();
const idPattern = /^[a-zA-Z0-9_-]{1,128}$/;
type Row = Record<string, any>;
async function open(): Promise<void> {
  if (history) return;
  if (opening) return opening;
  opening = (async () => {
    const { DatabaseSync } = await import('node:sqlite');
    fs.mkdirSync(getConfigDir(), {recursive:true});
    db = new DatabaseSync(path.join(getConfigDir(), 'headless-analytics.sqlite'));
    db.exec('PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;');
    db.exec('BEGIN IMMEDIATE');
    try {
      db.exec(MIGRATION_V2);
      if (!db.prepare('PRAGMA table_info(session_history)').all().some(row => row.name === 'provider')) db.exec(MIGRATION_V3);
      db.exec(MIGRATION_V4);
      db.exec('CREATE TABLE IF NOT EXISTS analytics_sources (session_id TEXT PRIMARY KEY, fingerprint TEXT NOT NULL)');
      db.exec('COMMIT');
    } catch (error) { db.exec('ROLLBACK'); throw error; }
    history = new SessionHistoryStore({db:{prepare(sql:string) {
      const statement = db.prepare(sql);
      // better-sqlite3 ignores unused named keys (the common record API).
      statement.setAllowUnknownNamedParameters(true);
      return statement;
    }}} as unknown as {db: Pick<BetterSqlite3.Database,'prepare'>}, true);
  })();
  try { await opening; } finally { opening = undefined; }
}
function number(...values: unknown[]): number {
  for (const v of values) if (typeof v === 'number' && Number.isFinite(v) && v >= 0) return v;
  return 0;
}
function iso(value: unknown): string {
  if (typeof value !== 'string' && typeof value !== 'number') return '';
  const date = new Date(value);
  return Number.isFinite(date.getTime()) ? date.toISOString() : '';
}
function transcriptIndex(roots: string[]): Map<string,string> {
  const out = new Map<string,string>();
  for (const root of roots) {
    let dirs: fs.Dirent[];
    try { dirs = fs.readdirSync(root,{withFileTypes:true}); } catch { continue; }
    for (const dir of dirs) {
      if (!dir.isDirectory()) continue;
      try { for (const file of fs.readdirSync(path.join(root,dir.name))) {
        if (!file.endsWith('.jsonl')) continue;
        out.set(file.slice(0,-6),path.join(root,dir.name,file));
      } } catch { /* A concurrently removed project is absent. */ }
    }
  }
  return out;
}
async function observe(rows: Row[]): Promise<void> {
  const roots = [process.env.CLAUDE_CONFIG_DIR || path.join(os.homedir(),'.claude'),...claudeProfiles.getProfiles().filter(p => !p.provider || p.provider === 'claude').map(p => p.configDir.replace(/^~/,os.homedir())).filter(Boolean)].map(root => path.join(root,'projects'));
  const transcripts = transcriptIndex(roots);
  for (const row of rows) {
    const id = row.sessionId ?? row.session_id;
    if (typeof id !== 'string' || !idPattern.test(id) || id.startsWith('agent-')) continue;
    const prior = db.prepare('SELECT * FROM session_history WHERE session_id=?').get(id);
    const provider = row.provider || 'claude';
    const sl = row.statusLine ?? row.status_line ?? {};
    const usage = row.usage ?? {};
    const model = usage.model || sl.modelDisplay || sl.model_display || sl.model || prior?.model || '';
    const stopped = row.status === 'ended' || row.mode === 'stopped';
    const start = iso(row.startedAt ?? row.started_at) || String(prior?.started_at ?? '');
    const updated = iso(row.updated_at ?? row.lastActivity) || String(prior?.ended_at ?? '');
    const end = stopped ? updated : '';
    const workflows = Array.isArray(row.workflows) ? row.workflows : [];
    const subagents = Array.isArray(row.subagents) ? row.subagents : [];
    const record: SessionHistoryRecord = {
      sessionId:id,cwd:row.cwd ?? '',agentName:row.label || path.basename(row.cwd || '') || id.slice(0,8),provider,model,
      gitBranch:sl.gitBranch ?? sl.git_branch ?? String(prior?.git_branch ?? ''),startedAt:start,endedAt:end,
      durationMs:start ? Math.max(0,(stopped ? Date.parse(end) || Date.parse(start) : Date.now())-Date.parse(start)) : 0,
      inputTokens:number(usage.totalInputTokens,sl.totalInputTokens,sl.total_input_tokens,prior?.input_tokens),
      outputTokens:number(usage.totalOutputTokens,sl.totalOutputTokens,sl.total_output_tokens,prior?.output_tokens),
      costUSD:provider === 'claude' ? number(prior?.cost_usd,usage.costUSD,usage.cost_usd,sl.costUSD,sl.cost_usd) : number(sl.costUSD,sl.cost_usd,prior?.cost_usd),
      peakContext:Math.max(number(row.peakContext,usage.contextTokens,usage.context_tokens),number(prior?.peak_context)),
      toolCalls:number(row.totalToolCalls,row.tool_calls,prior?.tool_calls),messageCount:number(row.messageCount,prior?.message_count),
      subagentCount:subagents.length + workflows.reduce((n:number,w:Row)=>n+(w.agents?.length ?? 0),0),
      workflowRuns:workflows.length || number(prior?.workflow_runs),workflowFailed:workflows.filter((w:Row)=>w.status==='failed').length,
      status:stopped ? 'ended' : 'active',
    };
    let models: Record<string,{inputTokens:number;outputTokens:number;costUSD:number}> | undefined;
    let fingerprint: string | undefined;
    if (provider === 'claude') {
      const candidate = row.transcriptPath ?? row.transcript_path ?? transcripts.get(id);
      if (typeof candidate === 'string') {
        const main = assertPathAllowed('analytics.summary',candidate,roots);
        const files = [main,...subagentFilesFor(main).map(file=>assertPathAllowed('analytics.summary',file,roots))];
        try {
          fingerprint = JSON.stringify(files.map(file=>{const s=fs.statSync(file);return [file,s.dev,s.ino,s.size,s.mtimeMs]}));
          const previous = db.prepare('SELECT fingerprint FROM analytics_sources WHERE session_id=?').get(id);
          if (fingerprint !== previous?.fingerprint) {
            const result = await recomputeSession(main,files.slice(1));
            if (result) {Object.assign(record,result);models=result.models;}
          } else if (prior) {
            record.inputTokens=Number(prior.input_tokens);record.outputTokens=Number(prior.output_tokens);
            record.costUSD=Number(prior.cost_usd);record.model=String(prior.model);record.peakContext=Number(prior.peak_context);
          }
        } catch (error) {
          if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error;
          fingerprint=undefined;
          // Retain already recorded totals when a transcript has been cleaned up.
          if (prior) {record.inputTokens=Number(prior.input_tokens);record.outputTokens=Number(prior.output_tokens);record.costUSD=Number(prior.cost_usd);}
        }
      }
    } else if (model) {
      models = {[model]:{inputTokens:record.inputTokens,outputTokens:record.outputTokens,costUSD:record.costUSD}};
    }
    db.exec('BEGIN IMMEDIATE');
    try {
      history.record(record);
      if (models) {
        db.prepare('DELETE FROM session_model_usage WHERE session_id=?').run(id);
        history.recordModels(id,models);
      }
      if (fingerprint) db.prepare('INSERT OR REPLACE INTO analytics_sources VALUES (?,?)').run(id,fingerprint);
      db.exec('COMMIT');
    } catch(error) {db.exec('ROLLBACK');throw error;}
  }
}
export function headlessAnalytics(method: string, params: Row, snapshots: Row[]): Promise<unknown> {
  // Serialize transcript folds and SQL transactions across concurrent requests.
  const result = tail.then(async () => {
    await open();
    await observe(snapshots);
    const provider = typeof params.provider === 'string' ? params.provider : undefined;
    const since = typeof params.since === 'string' ? iso(params.since) : undefined;
    if (params.since && !since) throw new Error('Invalid analytics time range');
    if (method === 'analytics.summary') return history.summary(provider,since);
    const limit = typeof params.limit === 'number' ? Math.max(1,Math.min(10_000,Math.floor(params.limit))) : 100;
    return history.recent(limit,provider,since);
  });
  tail=result.catch(()=>{});
  return result;
}
