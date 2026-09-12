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
 * v3 (2026-08-24) re-runs it for a third reason of the same shape: cache writes
 * were priced at a flat 1.25x the input rate, which is the 5-minute-TTL rate,
 * while these sessions ran on the 1-hour TTL that bills at 2x. Costs are
 * accumulated at write time here too, so fixing turnCostUSD does nothing for
 * rows already on disk. The transcripts carry the TTL split per turn, so a
 * re-derivation prices them correctly.
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
import { database } from './db';
import { sessionHistory } from './sessionHistory';
import { recomputeSession, subagentFilesFor, type RecomputedUsage } from './analyticsUsage';
export { recomputeSession, type RecomputedUsage } from './analyticsUsage';

// v2: re-run over v1 rows — v1 priced Opus 4.0 dated ids ('claude-opus-4-2…')
// at the generic Opus rate because the 'claude-opus-4-0' rate key never
// matched them, and it left stale session_model_usage slices behind (rows for
// model keys the recompute no longer produced survived and double-counted in
// the by-model analytics).
const BACKFILL_NAME = 'transcript-usage-v3';

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
