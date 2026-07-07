/**
 * One-shot analytics backfill: re-derive historical session usage straight
 * from the Claude Code transcripts on disk (~/.claude/projects/<dir>/<id>.jsonl).
 *
 * Rows written before 2026-07-06 have two problems this fixes:
 *   • subagent (isSidechain) turns were never counted — sessions that fanned
 *     out to Task agents under-report tokens and cost;
 *   • the pricing table was stale (Opus at Opus-3 rates, Fable priced as
 *     Sonnet) — costs were accumulated at write time, so the rows kept the
 *     wrong numbers even after the table was fixed.
 *
 * Recomputes tokens / cost / peak context / per-model split for every Claude
 * session whose transcript still exists, and rewrites session_history +
 * session_model_usage. Sessions whose transcripts Claude Code already cleaned
 * up keep their old (approximate) numbers. Runs once, marker-guarded in the
 * `_backfills` table — delete its row to force a re-run.
 */

import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import * as readline from 'readline';
import { database } from './db';
import { sessionHistory } from './sessionHistory';
import { contextTokensOf, turnCostUSD, type ModelUsageSlice } from './modelUsage';

// v2: re-run over v1 rows — v1 priced Opus 4.0 dated ids ('claude-opus-4-2…')
// at the generic Opus rate because the 'claude-opus-4-0' rate key never
// matched them, and it left stale session_model_usage slices behind (rows for
// model keys the recompute no longer produced survived and double-counted in
// the by-model analytics).
const BACKFILL_NAME = 'transcript-usage-v2';

export interface RecomputedUsage {
  /** Last main-thread model ('' when the transcript never named one). */
  model: string | null;
  inputTokens: number;
  outputTokens: number;
  costUSD: number;
  peakContext: number;
  models: Record<string, ModelUsageSlice>;
}

/**
 * Stream one transcript file into `out`, the same way the live accumulator
 * folds usage items: totals/cost across main + sidechain turns (each priced
 * at its own model), context/model from main-thread turns only, deduped by
 * message id. `forceSidechain` treats every row as a subagent turn — used for
 * the per-agent `subagents/*.jsonl` files, whose rows all belong to a
 * sub-agent's run. Returns true if the file carried any assistant usage.
 */
async function foldTranscriptFile(
  file: string,
  out: RecomputedUsage,
  seen: Set<string>,
  forceSidechain: boolean,
): Promise<boolean> {
  const rl = readline.createInterface({
    input: fs.createReadStream(file),
    crlfDelay: Infinity,
  });
  let any = false;

  for await (const line of rl) {
    if (!line.trim()) continue;
    let row: any;
    try {
      row = JSON.parse(line);
    } catch {
      continue;
    }
    if (row?.type !== 'assistant') continue;
    const msg = row.message;
    const usage = msg?.usage;
    if (!usage || typeof usage !== 'object') continue;
    any = true;

    const sidechain = forceSidechain || row.isSidechain === true;
    // "<synthetic>" is Claude Code's placeholder on synthetic messages, not a
    // real model — treat it as unnamed so it inherits the thread's model.
    let rowModel: string | null = typeof msg.model === 'string' ? msg.model : null;
    if (rowModel?.startsWith('<')) rowModel = null;

    // Context gauge / reported model: main thread only.
    if (!sidechain) {
      const ctx = contextTokensOf(usage);
      if (ctx > out.peakContext) out.peakContext = ctx;
      if (rowModel) out.model = rowModel;
    }

    // Cumulative — once per distinct message id (streamed blocks repeat it).
    const id =
      (typeof msg.id === 'string' && msg.id) || (typeof row.uuid === 'string' && row.uuid) || '';
    if (id) {
      if (seen.has(id)) continue;
      seen.add(id);
    }
    const turnModel = rowModel ?? out.model;
    const inputTokens = contextTokensOf(usage);
    const outputTokens = usage.output_tokens ?? 0;
    const costUSD = turnCostUSD(turnModel, usage);
    out.inputTokens += inputTokens;
    out.outputTokens += outputTokens;
    out.costUSD += costUSD;

    const slice = (out.models[turnModel ?? '(unknown)'] ??= {
      inputTokens: 0,
      outputTokens: 0,
      costUSD: 0,
    });
    slice.inputTokens += inputTokens;
    slice.outputTokens += outputTokens;
    slice.costUSD += costUSD;
  }

  return any;
}

/**
 * Recompute a session's usage from its main transcript plus any per-agent
 * `subagents/*.jsonl` files (where current Claude Code writes Task/teammate
 * agents). Returns null when nothing carried assistant usage.
 */
export async function recomputeSession(
  mainFile: string,
  subagentFiles: string[] = [],
): Promise<RecomputedUsage | null> {
  const seen = new Set<string>();
  const out: RecomputedUsage = {
    model: null,
    inputTokens: 0,
    outputTokens: 0,
    costUSD: 0,
    peakContext: 0,
    models: {},
  };
  let any = await foldTranscriptFile(mainFile, out, seen, false);
  for (const f of subagentFiles) {
    try {
      any = (await foldTranscriptFile(f, out, seen, true)) || any;
    } catch (err) {
      console.warn(`[AnalyticsBackfill] failed to parse subagent file ${f}:`, err);
    }
  }
  return any ? out : null;
}

/** Sub-agent transcript files for one session, next to its main transcript. */
function subagentFilesFor(mainFile: string): string[] {
  const dir = path.join(mainFile.replace(/\.jsonl$/, ''), 'subagents');
  try {
    return fs
      .readdirSync(dir)
      .filter((f) => f.endsWith('.jsonl'))
      .sort()
      .map((f) => path.join(dir, f));
  } catch {
    return [];
  }
}

/** Map session id → transcript path across every Claude Code project dir.
 *  Session ids are UUIDs, so a flat map can't collide across projects. */
function indexTranscripts(): Map<string, string> {
  const root = path.join(os.homedir(), '.claude', 'projects');
  const map = new Map<string, string>();
  let dirs: fs.Dirent[];
  try {
    dirs = fs.readdirSync(root, { withFileTypes: true });
  } catch {
    return map;
  }
  for (const d of dirs) {
    if (!d.isDirectory()) continue;
    const dir = path.join(root, d.name);
    let files: string[];
    try {
      files = fs.readdirSync(dir);
    } catch {
      continue;
    }
    for (const f of files) {
      if (f.endsWith('.jsonl')) map.set(f.slice(0, -'.jsonl'.length), path.join(dir, f));
    }
  }
  return map;
}

/**
 * Rewrite historical Claude rows from their transcripts. Idempotent and
 * marker-guarded — safe to call on every startup; only the first call works.
 */
export async function backfillAnalyticsFromTranscripts(): Promise<void> {
  const db = database.db;
  db.exec(
    `CREATE TABLE IF NOT EXISTS _backfills (name TEXT PRIMARY KEY, applied_at TEXT NOT NULL)`,
  );
  if (db.prepare('SELECT 1 FROM _backfills WHERE name=?').get(BACKFILL_NAME)) return;

  // Managed providers (codex/opencode/pi) have no Claude transcript to
  // re-derive from — only claude/legacy rows are candidates.
  const rows = db
    .prepare(
      `SELECT session_id AS sessionId, cost_usd AS costUSD
       FROM session_history WHERE provider='' OR provider='claude'`,
    )
    .all() as { sessionId: string; costUSD: number }[];
  const transcripts = indexTranscripts();
  const update = db.prepare(
    `UPDATE session_history SET
       model=@model, input_tokens=@inputTokens, output_tokens=@outputTokens,
       cost_usd=@costUSD, peak_context=@peakContext, updated_at=@updatedAt
     WHERE session_id=@sessionId`,
  );
  // The recompute re-attributes usage (e.g. live-recorded '(unknown)' slices
  // get a concrete model from the transcript), but recordModels only UPSERTS
  // the keys it is given — rows for keys the recompute no longer produces
  // would survive and double-count in the by-model analytics (summary() UNIONs
  // every session_model_usage row per session). Clear the session's split
  // first so the recomputed rows are the whole truth.
  const clearModels = db.prepare(`DELETE FROM session_model_usage WHERE session_id=?`);

  let updated = 0;
  let skipped = 0;
  let costBefore = 0;
  let costAfter = 0;
  for (const row of rows) {
    const file = transcripts.get(row.sessionId);
    if (!file) {
      skipped++;
      continue;
    }
    let re: RecomputedUsage | null = null;
    try {
      re = await recomputeSession(file, subagentFilesFor(file));
    } catch (err) {
      console.warn(`[AnalyticsBackfill] failed to parse ${file}:`, err);
    }
    if (!re) {
      skipped++;
      continue;
    }
    costBefore += row.costUSD;
    costAfter += re.costUSD;
    update.run({
      sessionId: row.sessionId,
      model: re.model ?? '',
      inputTokens: re.inputTokens,
      outputTokens: re.outputTokens,
      costUSD: re.costUSD,
      peakContext: re.peakContext,
      updatedAt: new Date().toISOString(),
    });
    clearModels.run(row.sessionId);
    sessionHistory.recordModels(row.sessionId, re.models);
    updated++;
  }

  db.prepare('INSERT INTO _backfills (name, applied_at) VALUES (?, ?)').run(
    BACKFILL_NAME,
    new Date().toISOString(),
  );
  console.log(
    `[AnalyticsBackfill] ${BACKFILL_NAME}: rewrote ${updated}/${rows.length} sessions ` +
      `(${skipped} kept as-is — transcript gone or empty); ` +
      `recorded cost $${costBefore.toFixed(2)} → $${costAfter.toFixed(2)}`,
  );
}
