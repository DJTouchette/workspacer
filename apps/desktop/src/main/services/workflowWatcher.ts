/**
 * Watches Claude Code's on-disk workflow + subagent artifacts and surfaces
 * live progress while a session runs. Pure filesystem tailing — no Claude
 * comms — so it lives beside `claudeSessionStore`'s transcript reader.
 *
 * On-disk layout (sessionDir = transcriptPath minus `.jsonl`):
 *   <sessionDir>/subagents/agent-<id>.jsonl            plain Agent-tool subagent transcript (appended live)
 *   <sessionDir>/subagents/agent-<id>.meta.json        { agentType, description?, toolUseId? }
 *   <sessionDir>/subagents/workflows/wf_<runId>/       one dir per Workflow run, created at run start
 *     agent-<id>.jsonl / agent-<id>.meta.json          workflow subagents (transcripts appended live)
 *     journal.jsonl                                    { type: started|result, agentId, ... } per agent
 *   <sessionDir>/workflows/scripts/<name>-wf_<id>.js   script copy, written at run start (meta = name/phases)
 *   <sessionDir>/workflows/wf_<runId>.json             rich final state — written ONCE at completion
 *
 * Live progress therefore comes from tailing the run dir; the final state
 * file is adopted as the authoritative record when it appears.
 */
import * as fs from 'fs';
import * as path from 'path';
import vm from 'node:vm';
import { turnCostUSD } from './modelUsage';

// ── Public types (mirrored in src/renderer/src/types/claudeSession.ts) ──

export interface WorkflowPhaseInfo {
  title: string;
  detail?: string;
}

export interface WorkflowAgentInfo {
  id: string;
  label?: string; // authoritative label — only known once the final state file lands
  phaseTitle?: string;
  model?: string;
  status: 'queued' | 'running' | 'done' | 'failed';
  startedAt?: number;
  completedAt?: number;
  durationMs?: number;
  tokens: number; // live: cumulative output tokens; final: authoritative figure
  /** Estimated USD cost, accumulated live from the agent's usage blocks (modelUsage rates). */
  costUSD?: number;
  toolCalls: number;
  lastToolName?: string;
  lastToolSummary?: string;
  promptPreview?: string;
  resultPreview?: string;
}

export interface WorkflowRunInfo {
  runId: string;
  name?: string;
  description?: string;
  status: 'running' | 'completed' | 'failed';
  startedAt: number;
  completedAt?: number;
  durationMs?: number;
  phases: WorkflowPhaseInfo[];
  agents: WorkflowAgentInfo[];
  totalTokens?: number;
  totalToolCalls?: number;
  /** Estimated USD cost — sum of the agents' live-accumulated costs. */
  totalCostUSD?: number;
}

/** Live enrichment for a plain (Agent-tool) subagent, joined by agent id. */
export interface SubagentActivity {
  description?: string;
  agentType?: string;
  /** The Agent tool_use id that spawned this subagent (from meta.json) — lets the renderer anchor it exactly. */
  toolUseId?: string;
  model?: string;
  tokens?: number;
  costUSD?: number;
  toolCalls?: number;
  lastToolName?: string;
  lastToolSummary?: string;
}

export interface WorkflowWatcherUpdate {
  runs: WorkflowRunInfo[];
  subagentActivity: Record<string, SubagentActivity>;
  /** Ids of agents that belong to workflow runs — used to keep them out of the plain subagent list. */
  workflowAgentIds: string[];
}

// ── Internals ──

const TICK_MS = 2500;
/** Stop ticking this long after the last hook-event poke (unless a run is live). */
const IDLE_AFTER_MS = 60_000;
/** Most-recent runs kept in the snapshot. */
const MAX_RUNS = 3;

interface TailState {
  offset: number;
  remainder: string;
}

interface AgentState {
  info: WorkflowAgentInfo;
  tail: TailState;
  lastUsageKey: string | null;
  lastMtimeMs: number;
}

interface RunState {
  info: WorkflowRunInfo;
  dir: string;
  agents: Map<string, AgentState>; // id without `agent-` prefix
  journalTail: TailState;
  finalized: boolean;
}

interface PlainAgentState {
  activity: SubagentActivity;
  tail: TailState;
  lastUsageKey: string | null;
  lastMtimeMs: number;
}

interface SessionWatch {
  sessionId: string;
  sessionDir: string;
  runs: Map<string, RunState>;
  plainAgents: Map<string, PlainAgentState>;
  timer: NodeJS.Timeout | null;
  lastPoke: number;
  dirty: boolean;
  onUpdate: (update: WorkflowWatcherUpdate) => void;
}

const stripAgentPrefix = (s: string): string => s.replace(/^agent-/, '');

function tailLines(filePath: string, t: TailState): string[] {
  let fd: number;
  try {
    fd = fs.openSync(filePath, 'r');
  } catch {
    return [];
  }
  try {
    const stat = fs.fstatSync(fd);
    if (stat.size <= t.offset) return [];
    const len = stat.size - t.offset;
    const buf = Buffer.alloc(len);
    fs.readSync(fd, buf, 0, len, t.offset);
    t.offset = stat.size;
    const text = t.remainder + buf.toString('utf8');
    const lines = text.split('\n');
    t.remainder = lines.pop() ?? '';
    return lines.filter(l => l.trim());
  } catch {
    return [];
  } finally {
    fs.closeSync(fd);
  }
}

function readJsonSafe(filePath: string): any | null {
  try {
    return JSON.parse(fs.readFileSync(filePath, 'utf8'));
  } catch {
    return null;
  }
}

/** Short human summary of a tool invocation, for the "last activity" line. */
function summarizeToolInput(name: string, input: any): string {
  if (!input || typeof input !== 'object') return '';
  const base = (p: unknown) => (typeof p === 'string' ? p.split(/[/\\]/).pop() ?? '' : '');
  switch (name) {
    case 'Read':
    case 'Edit':
    case 'MultiEdit':
    case 'Write':
    case 'NotebookEdit':
      return base(input.file_path);
    case 'Bash':
    case 'PowerShell':
      return String(input.description ?? input.command ?? '').split('\n')[0].slice(0, 60);
    case 'Grep':
    case 'Glob':
      return String(input.pattern ?? '').slice(0, 60);
    case 'Agent':
      return String(input.description ?? '').slice(0, 60);
    default: {
      const v = Object.values(input).find(x => typeof x === 'string') as string | undefined;
      return (v ?? '').split('\n')[0].slice(0, 60);
    }
  }
}

/**
 * Extract the `export const meta = {...}` literal from a persisted workflow
 * script. The Workflow contract requires meta to be a pure literal, so
 * evaluating just that object expression is safe; fall back to filename-derived
 * name on any failure.
 */
function parseScriptMeta(scriptText: string): { name?: string; description?: string; phases?: WorkflowPhaseInfo[] } | null {
  const m = /export\s+const\s+meta\s*=\s*\{/.exec(scriptText);
  if (!m) return null;
  const start = scriptText.indexOf('{', m.index);
  let depth = 0;
  let end = -1;
  let inStr: string | null = null;
  for (let i = start; i < scriptText.length; i++) {
    const c = scriptText[i];
    if (inStr) {
      if (c === '\\') i++;
      else if (c === inStr) inStr = null;
      continue;
    }
    if (c === '"' || c === "'" || c === '`') inStr = c;
    else if (c === '{') depth++;
    else if (c === '}') {
      depth--;
      if (depth === 0) { end = i; break; }
    }
  }
  if (end === -1) return null;
  try {
    const literalText = scriptText.slice(start, end + 1);
    const obj = vm.runInNewContext('(' + literalText + ')', Object.create(null), { timeout: 50, microtaskMode: 'afterEvaluate' });
    if (!obj || typeof obj !== 'object') return null;
    return {
      name: typeof obj.name === 'string' ? obj.name : undefined,
      description: typeof obj.description === 'string' ? obj.description : undefined,
      phases: Array.isArray(obj.phases)
        ? obj.phases
            .filter((p: any) => p && typeof p.title === 'string')
            .map((p: any) => ({ title: p.title, detail: typeof p.detail === 'string' ? p.detail : undefined }))
        : undefined,
    };
  } catch {
    return null;
  }
}

/** Fold one transcript JSONL entry into live per-agent stats. */
function applyTranscriptEntry(
  entry: any,
  stats: { tokens: number; costUSD?: number; toolCalls: number; model?: string; lastToolName?: string; lastToolSummary?: string; promptPreview?: string; startedAt?: number },
  usageDedup: { lastUsageKey: string | null },
): void {
  const ts = entry.timestamp ? Date.parse(entry.timestamp) : NaN;
  if (!stats.startedAt && !Number.isNaN(ts)) stats.startedAt = ts;

  const msg = entry.message;
  if (!msg) return;

  if (entry.type === 'user' && !stats.promptPreview) {
    const content = typeof msg.content === 'string'
      ? msg.content
      : Array.isArray(msg.content)
        ? msg.content.filter((b: any) => b.type === 'text').map((b: any) => b.text).join('\n')
        : '';
    if (content) stats.promptPreview = content.slice(0, 160);
  } else if (entry.type === 'assistant') {
    if (msg.model) stats.model = msg.model;
    if (msg.usage) {
      const key = msg.id ?? entry.uuid ?? null;
      if (key && key !== usageDedup.lastUsageKey) {
        usageDedup.lastUsageKey = key;
        stats.tokens += msg.usage.output_tokens ?? 0;
        stats.costUSD = (stats.costUSD ?? 0) + turnCostUSD(msg.model ?? stats.model, msg.usage);
      }
    }
    const blocks = Array.isArray(msg.content) ? msg.content : [];
    for (const b of blocks) {
      if (b.type === 'tool_use') {
        stats.toolCalls += 1;
        stats.lastToolName = b.name ?? 'tool';
        stats.lastToolSummary = summarizeToolInput(b.name, b.input);
      }
    }
  }
}

// ── Watcher ──

class WorkflowWatcher {
  private watches = new Map<string, SessionWatch>();

  /** Begin watching a session. Idempotent; safe to call once transcriptPath is known. */
  attach(sessionId: string, transcriptPath: string, onUpdate: (update: WorkflowWatcherUpdate) => void): void {
    if (this.watches.has(sessionId)) return;
    const sessionDir = transcriptPath.replace(/\.jsonl$/i, '');
    if (sessionDir === transcriptPath) return; // unexpected path shape
    const watch: SessionWatch = {
      sessionId,
      sessionDir,
      runs: new Map(),
      plainAgents: new Map(),
      timer: null,
      lastPoke: Date.now(),
      dirty: false,
      onUpdate,
    };
    this.watches.set(sessionId, watch);
    this.ensureTimer(watch);
  }

  /** Hook-event heartbeat — keeps the poll loop alive while the session is busy. */
  poke(sessionId: string): void {
    const watch = this.watches.get(sessionId);
    if (!watch) return;
    watch.lastPoke = Date.now();
    this.ensureTimer(watch);
  }

  detach(sessionId: string): void {
    const watch = this.watches.get(sessionId);
    if (!watch) return;
    if (watch.timer) clearInterval(watch.timer);
    this.watches.delete(sessionId);
  }

  detachAll(): void {
    for (const id of Array.from(this.watches.keys())) this.detach(id);
  }

  /**
   * Read a subagent's transcript for the drill-in view. `runId` names a
   * workflow run (file resolved from the run's known dir); pass null for a
   * plain Agent-tool subagent (file lives in the session's subagents/ root).
   * Returns lightweight turns (role + text, with tool calls flattened to
   * one-line summaries) — enough to read what the agent did without shipping
   * the raw JSONL. `null` if the session/run/file is gone.
   */
  readAgentTranscript(sessionId: string, runId: string | null, agentId: string): { role: string; text: string }[] | null {
    const watch = this.watches.get(sessionId);
    if (!watch) return null;
    const dir = runId ? watch.runs.get(runId)?.dir : path.join(watch.sessionDir, 'subagents');
    if (!dir) return null;
    const file = path.join(dir, `agent-${stripAgentPrefix(agentId)}.jsonl`);
    let raw: string;
    try {
      raw = fs.readFileSync(file, 'utf8');
    } catch {
      return null;
    }
    const turns: { role: string; text: string }[] = [];
    for (const line of raw.split('\n')) {
      if (!line.trim()) continue;
      let j: any;
      try { j = JSON.parse(line); } catch { continue; }
      const msg = j.message ?? j;
      const role = msg.role ?? j.type;
      if (role !== 'user' && role !== 'assistant') continue;
      const parts: string[] = [];
      const content = msg.content;
      if (typeof content === 'string') {
        parts.push(content);
      } else if (Array.isArray(content)) {
        for (const b of content) {
          if (b?.type === 'text' && typeof b.text === 'string') parts.push(b.text);
          else if (b?.type === 'tool_use') parts.push(`⚙ ${b.name ?? 'tool'}`);
          else if (b?.type === 'tool_result') {
            const t = typeof b.content === 'string' ? b.content
              : Array.isArray(b.content) ? b.content.map((c: any) => (typeof c?.text === 'string' ? c.text : '')).join('') : '';
            if (t.trim()) parts.push(`↳ ${t.slice(0, 400)}`);
          }
        }
      }
      const text = parts.join('\n').trim();
      if (text) turns.push({ role, text });
    }
    return turns;
  }

  // ── Poll loop ──

  private ensureTimer(watch: SessionWatch): void {
    if (watch.timer) return;
    watch.timer = setInterval(() => this.tick(watch), TICK_MS);
    watch.timer.unref();
    // Run one immediately so a fresh attach/poke reflects state without delay
    this.tick(watch);
  }

  private tick(watch: SessionWatch): void {
    try {
      this.scanRuns(watch);
      this.scanPlainAgents(watch);
    } catch (err) {
      console.error('[WorkflowWatcher] tick error:', err);
    }

    if (watch.dirty) {
      watch.dirty = false;
      watch.onUpdate(this.buildUpdate(watch));
    }

    // Idle out when nothing is live and hooks have gone quiet; poke() revives us.
    const hasLiveRun = Array.from(watch.runs.values()).some(r => !r.finalized);
    if (!hasLiveRun && Date.now() - watch.lastPoke > IDLE_AFTER_MS && watch.timer) {
      clearInterval(watch.timer);
      watch.timer = null;
    }
  }

  private buildUpdate(watch: SessionWatch): WorkflowWatcherUpdate {
    const runs = Array.from(watch.runs.values())
      .map(r => r.info)
      .sort((a, b) => a.startedAt - b.startedAt)
      .slice(-MAX_RUNS)
      // Deep-ish copy so IPC snapshots don't share mutable state
      .map(r => {
        const agents = r.agents.map(a => ({ ...a }));
        // Run cost is derivable only from the live per-agent accumulation (the
        // final state file doesn't carry cost), so roll it up at snapshot time.
        const cost = agents.reduce((s, a) => s + (a.costUSD ?? 0), 0);
        return { ...r, phases: r.phases.map(p => ({ ...p })), agents, totalCostUSD: cost > 0 ? cost : r.totalCostUSD };
      });

    const subagentActivity: Record<string, SubagentActivity> = {};
    for (const [id, st] of watch.plainAgents) subagentActivity[id] = { ...st.activity };

    // Mirror the MAX_RUNS-sliced `runs` — agents of runs dropped from the
    // snapshot must not stay suppressed (they'd vanish from both the run cards
    // and the plain subagent list).
    const workflowAgentIds: string[] = runs.flatMap(r => r.agents.map(a => a.id));
    return { runs, subagentActivity, workflowAgentIds };
  }

  // ── Workflow runs ──

  private scanRuns(watch: SessionWatch): void {
    const runsRoot = path.join(watch.sessionDir, 'subagents', 'workflows');
    let entries: fs.Dirent[] = [];
    try {
      entries = fs.readdirSync(runsRoot, { withFileTypes: true });
    } catch {
      return; // no workflows yet
    }

    for (const e of entries) {
      if (!e.isDirectory() || !e.name.startsWith('wf_')) continue;
      if (!watch.runs.has(e.name)) {
        this.registerRun(watch, e.name, path.join(runsRoot, e.name));
      }
    }

    for (const run of watch.runs.values()) {
      if (run.finalized) continue;
      if (this.tryAdoptFinal(watch, run)) continue;
      this.refreshRunAgents(watch, run);
      this.refreshJournal(watch, run);
    }
  }

  private registerRun(watch: SessionWatch, runId: string, dir: string): void {
    let startedAt = Date.now();
    try {
      const st = fs.statSync(dir);
      startedAt = st.birthtimeMs || st.mtimeMs;
    } catch {}

    const run: RunState = {
      dir,
      agents: new Map(),
      journalTail: { offset: 0, remainder: '' },
      finalized: false,
      info: {
        runId,
        status: 'running',
        startedAt,
        phases: [],
        agents: [],
      },
    };

    // Script copy is written at run start: <sessionDir>/workflows/scripts/<name>-<runId>.js
    try {
      const scriptsRoot = path.join(watch.sessionDir, 'workflows', 'scripts');
      const scriptFile = fs.readdirSync(scriptsRoot).find(f => f.endsWith(`-${runId}.js`));
      if (scriptFile) {
        run.info.name = scriptFile.slice(0, -(`-${runId}.js`.length));
        const meta = parseScriptMeta(fs.readFileSync(path.join(scriptsRoot, scriptFile), 'utf8'));
        if (meta) {
          if (meta.name) run.info.name = meta.name;
          run.info.description = meta.description;
          if (meta.phases) run.info.phases = meta.phases;
        }
      }
    } catch {}

    watch.runs.set(runId, run);
    watch.dirty = true;
  }

  /** The final wf_<runId>.json is authoritative — adopt it and stop tailing. */
  private tryAdoptFinal(watch: SessionWatch, run: RunState): boolean {
    const finalPath = path.join(watch.sessionDir, 'workflows', `${run.info.runId}.json`);
    if (!fs.existsSync(finalPath)) return false;
    const j = readJsonSafe(finalPath);
    if (!j) return false;

    const info = run.info;
    info.status = j.status === 'completed' ? 'completed' : 'failed';
    if (typeof j.workflowName === 'string') info.name = j.workflowName;
    if (typeof j.startTime === 'number') info.startedAt = j.startTime;
    if (typeof j.durationMs === 'number') {
      info.durationMs = j.durationMs;
      info.completedAt = info.startedAt + j.durationMs;
    }
    if (Array.isArray(j.phases)) {
      info.phases = j.phases
        .filter((p: any) => p && typeof p.title === 'string')
        .map((p: any) => ({ title: p.title, detail: typeof p.detail === 'string' ? p.detail : undefined }));
    }
    if (typeof j.totalTokens === 'number') info.totalTokens = j.totalTokens;
    if (typeof j.totalToolCalls === 'number') info.totalToolCalls = j.totalToolCalls;

    if (Array.isArray(j.workflowProgress)) {
      const agents: WorkflowAgentInfo[] = [];
      for (const p of j.workflowProgress) {
        if (p?.type !== 'workflow_agent' || !p.agentId) continue;
        // The final file has no cost figures — keep what the live tail accumulated.
        const liveCost = run.agents.get(stripAgentPrefix(String(p.agentId)))?.info.costUSD;
        agents.push({
          id: stripAgentPrefix(String(p.agentId)),
          costUSD: liveCost,
          label: typeof p.label === 'string' ? p.label : undefined,
          phaseTitle: typeof p.phaseTitle === 'string' ? p.phaseTitle : undefined,
          model: typeof p.model === 'string' ? p.model : undefined,
          status: p.state === 'done' ? 'done' : p.state === 'error' || p.state === 'failed' ? 'failed' : 'done',
          startedAt: typeof p.startedAt === 'number' ? p.startedAt : undefined,
          durationMs: typeof p.durationMs === 'number' ? p.durationMs : undefined,
          completedAt:
            typeof p.startedAt === 'number' && typeof p.durationMs === 'number'
              ? p.startedAt + p.durationMs
              : undefined,
          tokens: typeof p.tokens === 'number' ? p.tokens : 0,
          toolCalls: typeof p.toolCalls === 'number' ? p.toolCalls : 0,
          lastToolName: typeof p.lastToolName === 'string' ? p.lastToolName : undefined,
          lastToolSummary: typeof p.lastToolSummary === 'string' ? p.lastToolSummary : undefined,
          promptPreview: typeof p.promptPreview === 'string' ? p.promptPreview.slice(0, 160) : undefined,
          resultPreview: typeof p.resultPreview === 'string' ? p.resultPreview.slice(0, 200) : undefined,
        });
      }
      if (agents.length > 0) {
        info.agents = agents;
        run.agents.clear();
        for (const a of agents) {
          run.agents.set(a.id, { info: a, tail: { offset: 0, remainder: '' }, lastUsageKey: null, lastMtimeMs: 0 });
        }
      }
    }

    run.finalized = true;
    watch.dirty = true;
    return true;
  }

  private refreshRunAgents(watch: SessionWatch, run: RunState): void {
    let files: string[] = [];
    try {
      files = fs.readdirSync(run.dir);
    } catch {
      return;
    }

    // meta.json appears at agent spawn — register
    for (const f of files) {
      const m = /^agent-(.+)\.meta\.json$/.exec(f);
      if (!m) continue;
      const id = m[1];
      if (run.agents.has(id)) continue;
      const agent: AgentState = {
        info: { id, status: 'queued', tokens: 0, toolCalls: 0 },
        tail: { offset: 0, remainder: '' },
        lastUsageKey: null,
        lastMtimeMs: 0,
      };
      run.agents.set(id, agent);
      run.info.agents.push(agent.info);
      watch.dirty = true;
    }

    // tail each agent transcript that has grown
    for (const [id, agent] of run.agents) {
      const jsonl = path.join(run.dir, `agent-${id}.jsonl`);
      let mtimeMs = 0;
      try {
        mtimeMs = fs.statSync(jsonl).mtimeMs;
      } catch {
        continue; // not started writing yet
      }
      if (mtimeMs === agent.lastMtimeMs) continue;
      agent.lastMtimeMs = mtimeMs;

      const lines = tailLines(jsonl, agent.tail);
      if (lines.length === 0) continue;
      if (agent.info.status === 'queued') agent.info.status = 'running';
      const dedup = { lastUsageKey: agent.lastUsageKey };
      for (const line of lines) {
        try {
          applyTranscriptEntry(JSON.parse(line), agent.info, dedup);
        } catch {
          // partial line shouldn't happen (tailLines keeps remainder), skip junk
        }
      }
      agent.lastUsageKey = dedup.lastUsageKey;
      watch.dirty = true;
    }
  }

  private refreshJournal(watch: SessionWatch, run: RunState): void {
    const journal = path.join(run.dir, 'journal.jsonl');
    const lines = tailLines(journal, run.journalTail);
    for (const line of lines) {
      let entry: any;
      try {
        entry = JSON.parse(line);
      } catch {
        continue;
      }
      const id = entry.agentId ? stripAgentPrefix(String(entry.agentId)) : null;
      if (!id) continue;
      const agent = run.agents.get(id);
      if (!agent) continue;
      if (entry.type === 'started' && agent.info.status === 'queued') {
        agent.info.status = 'running';
        watch.dirty = true;
      } else if (entry.type === 'result') {
        agent.info.status = 'done';
        agent.info.completedAt = Date.now();
        if (agent.info.startedAt) agent.info.durationMs = agent.info.completedAt - agent.info.startedAt;
        if (entry.result !== undefined && agent.info.resultPreview === undefined) {
          try {
            const s = typeof entry.result === 'string' ? entry.result : JSON.stringify(entry.result);
            agent.info.resultPreview = s.slice(0, 200);
          } catch {}
        }
        watch.dirty = true;
      }
    }
  }

  // ── Plain (Agent-tool) subagents ──

  private scanPlainAgents(watch: SessionWatch): void {
    const root = path.join(watch.sessionDir, 'subagents');
    let files: string[] = [];
    try {
      files = fs.readdirSync(root);
    } catch {
      return;
    }

    for (const f of files) {
      const m = /^agent-(.+)\.meta\.json$/.exec(f);
      if (!m) continue;
      const id = m[1];
      if (watch.plainAgents.has(id)) continue;
      const meta = readJsonSafe(path.join(root, f)) ?? {};
      watch.plainAgents.set(id, {
        activity: {
          agentType: typeof meta.agentType === 'string' ? meta.agentType : undefined,
          description: typeof meta.description === 'string' ? meta.description : undefined,
          toolUseId: typeof meta.toolUseId === 'string' ? meta.toolUseId : undefined,
          tokens: 0,
          toolCalls: 0,
        },
        tail: { offset: 0, remainder: '' },
        lastUsageKey: null,
        lastMtimeMs: 0,
      });
      watch.dirty = true;
    }

    for (const [id, st] of watch.plainAgents) {
      const jsonl = path.join(root, `agent-${id}.jsonl`);
      let mtimeMs = 0;
      try {
        mtimeMs = fs.statSync(jsonl).mtimeMs;
      } catch {
        continue;
      }
      if (mtimeMs === st.lastMtimeMs) continue;
      st.lastMtimeMs = mtimeMs;

      const lines = tailLines(jsonl, st.tail);
      if (lines.length === 0) continue;
      const stats = {
        tokens: st.activity.tokens ?? 0,
        costUSD: st.activity.costUSD,
        toolCalls: st.activity.toolCalls ?? 0,
        model: st.activity.model,
        lastToolName: st.activity.lastToolName,
        lastToolSummary: st.activity.lastToolSummary,
        promptPreview: undefined as string | undefined,
        startedAt: undefined as number | undefined,
      };
      const dedup = { lastUsageKey: st.lastUsageKey };
      for (const line of lines) {
        try {
          applyTranscriptEntry(JSON.parse(line), stats, dedup);
        } catch {}
      }
      st.lastUsageKey = dedup.lastUsageKey;
      st.activity.tokens = stats.tokens;
      st.activity.costUSD = stats.costUSD;
      st.activity.toolCalls = stats.toolCalls;
      st.activity.model = stats.model;
      st.activity.lastToolName = stats.lastToolName;
      st.activity.lastToolSummary = stats.lastToolSummary;
      watch.dirty = true;
    }
  }
}

export const workflowWatcher = new WorkflowWatcher();
