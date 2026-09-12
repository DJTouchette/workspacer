/** Shared streaming transcript accounting for native and headless history. */
import * as fs from 'fs';
import * as path from 'path';
import * as readline from 'readline';
import { contextTokensOf, turnCostUSD, type ModelUsageSlice } from './modelUsage';
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
export function subagentFilesFor(mainFile: string): string[] {
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
