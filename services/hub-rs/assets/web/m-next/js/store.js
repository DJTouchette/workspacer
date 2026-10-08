// Everything the screens render from: live session snapshots (local and
// federated), the shared archive, the host's kept-open set, conversations,
// usage, jobs, nodes and config. Ported from /m's data layer — the fold,
// federation routing, peer seeds/tombstones, archive ordering and the
// conversation polling fallback behave exactly as they did there.
//
// Views never call the bus for data directly; they read here and call
// `changed()` listeners via `onChange`.
import { bus, call, can, on as onBus } from './bus.js';
import { basename, hash, errText } from './util.js';

const listeners = new Set();
export const onChange = (fn) => { listeners.add(fn); return () => listeners.delete(fn); };
let scheduled = false;
const changedIds = new Set();
/** Coalesce bursts of events into one render per frame. */
export function changed(id) {
  changedIds.add(id || '*');
  if (scheduled) return;
  scheduled = true;
  requestAnimationFrame(() => {
    scheduled = false;
    const ids = new Set(changedIds);
    changedIds.clear();
    for (const fn of listeners) {
      try { fn(ids); } catch (e) { console.error('[store]', e); }
    }
  });
}

/** Live snapshots keyed by sessionId — the one source every screen renders from. */
export const sessions = new Map();

// ── federation ─────────────────────────────────────────────────────────────
// Linked hubs republish their agents' events with the peer's name on the
// ENVELOPE (absent = local); `hub:<peer>/<method>` routes a call back over the
// link. A hub whose link is down keeps its rows as read-only tombstones.
export const sessionHub = new Map();   // sessionId -> peer hub name
export const offlineHubs = new Map();  // peer hub name -> last-seen ts
export const hubOf = (s) => (s && s.hub) || '';
export const hubDown = (s) => { const h = hubOf(s); return !!h && offlineHubs.has(h); };
export const hubGone = (id) => { const h = sessionHub.get(id); return !!h && offlineHubs.has(h); };
/** The bus applies the tier check to the BARE method either way. */
export const qualify = (id, method) => { const h = sessionHub.get(id); return h ? `hub:${h}/${method}` : method; };
export let peers = [];

// Two providers serve snapshots: rich desktop rows and sparse headless rows.
// A sparse row must never clobber detail we already hold — merge it over.
function fold(snap) {
  const prev = sessions.get(snap.sessionId);
  const next = snap.sparse && prev ? Object.assign({}, prev, snap) : snap;
  sessions.set(snap.sessionId, next);
}
export function upsert(snap) {
  if (!snap || !snap.sessionId) return;
  fold(snap);
  changed(snap.sessionId);
}

export let seeded = false;
export async function seed() {
  try {
    const list = await call('sessions.snapshots', {});
    const seen = new Set();
    for (const s of list || []) {
      if (s && s.sessionId) { seen.add(s.sessionId); sessionHub.delete(s.sessionId); fold(s); }
    }
    await seedPeers(seen);
    await seedKept(seen);
    // The providers' lists ARE the visible fleet — drop what they no longer
    // show. A down peer answered nothing: its rows are tombstones to keep.
    for (const id of [...sessions.keys()]) {
      if (seen.has(id)) continue;
      const h = sessionHub.get(id);
      if (h && offlineHubs.has(h)) continue;
      sessions.delete(id); sessionHub.delete(id);
    }
    pruneConv();
    seeded = true;
    changed();
  } catch { /* the hub line shows offline */ }
}
async function seedPeers(seen) {
  let list = [];
  try { list = (await call('federation.peers', {})) || []; } catch { peers = []; return; }
  peers = list.filter((p) => p && p.name);
  const names = new Set();
  for (const p of peers) {
    names.add(p.name);
    if (p.connected) offlineHubs.delete(p.name);
    else if (!offlineHubs.has(p.name)) offlineHubs.set(p.name, p.lastSeen || 0);
  }
  // An UNLINKED hub is gone, not offline: its tombstones go with the link.
  for (const h of [...offlineHubs.keys()]) if (!names.has(h)) offlineHubs.delete(h);
  await Promise.all(peers.filter((p) => p.connected).map(async (p) => {
    try {
      const rows = (await call(`hub:${p.name}/sessions.snapshots`, {})) || [];
      for (const row of rows) {
        if (!row || !row.sessionId) continue;
        row.hub = p.name;
        seen.add(row.sessionId);
        sessionHub.set(row.sessionId, p.name);
        fold(row);
      }
    } catch {
      // A blip on a nominally-connected link must not wipe the peer's rows.
      for (const [id, h] of sessionHub) if (h === p.name) seen.add(id);
    }
  }));
}
/** A peer's link came back: replace its fleet wholesale. */
async function reseedPeer(name) {
  try {
    const rows = (await call(`hub:${name}/sessions.snapshots`, {})) || [];
    const seen = new Set();
    for (const row of rows) {
      if (!row || !row.sessionId) continue;
      row.hub = name;
      seen.add(row.sessionId);
      sessionHub.set(row.sessionId, name);
      fold(row);
    }
    for (const [id, h] of [...sessionHub]) {
      if (h === name && !seen.has(id)) { sessions.delete(id); sessionHub.delete(id); }
    }
  } catch { /* the 25s reconcile retries */ }
  changed();
}

// ── kept-open sessions (Paused) ───────────────────────────────────────────
// The native host remembers which sessions it had open when it closed
// (Settings::kept_open); the hub exposes that set as `sessions.kept`. The
// phone keeps no list of its own. A kept session the fleet list has aged out
// is read by id, as native does.
export const kept = new Map();          // sessionId -> since (ms)
export let keptSupported = false;
async function seedKept(seen) {
  if (!can('sessions.kept')) { keptSupported = false; return; }
  let rows = [];
  try {
    const r = await call('sessions.kept', {});
    rows = (r && Array.isArray(r.sessions)) ? r.sessions : [];
    keptSupported = true;
  } catch { keptSupported = false; kept.clear(); return; }
  kept.clear();
  for (const k of rows) if (k && k.sessionId) kept.set(k.sessionId, k.since || 0);
  await Promise.all([...kept.keys()].filter((id) => !seen.has(id)).map(async (id) => {
    try {
      const row = await call('sessions.snapshot', { sessionId: id });
      if (row && row.sessionId === id) { seen.add(id); fold(row); }
    } catch { /* gone from the hub too */ }
  }));
}

// ── session state ─────────────────────────────────────────────────────────
export const isLive = (s) => !!s && s.status !== 'ended' && s.mode !== 'stopped';
export const isBlocked = (s) => !!(s && (s.pendingApproval || (s.pendingQuestions && s.pendingQuestions.length)));
const WORKING = new Set(['streaming', 'thinking', 'responding', 'working', 'running']);
export const isWorking = (s) => {
  // A down hub's snapshots are frozen at the moment the link dropped.
  if (hubDown(s)) return false;
  return isLive(s) && !isBlocked(s) && (WORKING.has(s.ambientState) || WORKING.has(s.mode));
};
/** Stopped by the host closing rather than ended: resumable from the composer. */
export const isPaused = (s) =>
  !!s && !isLive(s) && kept.has(s.sessionId) && ['claude', 'codex', ''].includes(s.provider || '');
/** A Claude or Codex session that has stopped can be resumed by sending. */
export const canResume = (s) => !!s && !isLive(s) && ['claude', 'codex', ''].includes(s.provider || '') && !!s.cwd;
export const providerOf = (s) => (s && s.provider) || 'claude';

/** Native's status vocabulary (ui.rs session_status, kept.rs status_of). */
export function status(s) {
  if (hubDown(s)) return { label: 'Offline', tone: 'muted', key: 'offline' };
  if (!isLive(s)) return isPaused(s) ? { label: 'Paused', tone: 'accent', key: 'paused' } : { label: 'Ended', tone: 'muted', key: 'ended' };
  if (s.pendingApproval) return { label: 'Needs approval', tone: 'warning', key: 'approval' };
  if (s.pendingQuestions && s.pendingQuestions.length) return { label: 'Needs your input', tone: 'warning', key: 'question' };
  const a = s.ambientState || s.mode || '';
  if (WORKING.has(a)) return { label: 'Working', tone: 'busy', key: 'working' };
  if (a === 'waiting_approval' || a === 'approval') return { label: 'Needs approval', tone: 'warning', key: 'approval' };
  if (a === 'waiting_input' || a === 'question') return { label: 'Needs your input', tone: 'warning', key: 'question' };
  if (a === 'starting' || a === 'initializing') return { label: 'Starting', tone: 'muted', key: 'starting' };
  if (a === 'background') return { label: 'Background', tone: 'muted', key: 'background' };
  if (a === 'idle' || a === 'input') return { label: 'Ready', tone: 'success', key: 'ready' };
  return { label: 'Session', tone: 'muted', key: 'other' };
}
export const needsYou = (s) => isLive(s) && !hubDown(s) && isBlocked(s);

/** The row title: the launch label (or hub auto-title), else the folder. */
export const titleOf = (s) => (s && (s.label || s.customName || s.name || s.title)) || basename(s && s.cwd) ||
  String((s && s.sessionId) || '').slice(0, 8) || 'Session';
export const projectOf = (s) => basename((s && s.cwd) || '');

/** The model this session runs: the requested selection, else the runtime's. */
export function modelOf(s) {
  if (!s) return '';
  return (s.requestedSelection && s.requestedSelection.model) || (s.settings && s.settings.model) ||
    (s.usage && s.usage.model) || (s.statusLine && s.statusLine.modelDisplay) || s.model || '';
}
export const effortOf = (s) => (s && ((s.settings && s.settings.effort) || s.effort || (s.statusLine && s.statusLine.effort))) || '';
export const permissionOf = (s) => (s && (s.livePermissionMode || (s.settings && s.settings.permissionMode))) || '';

/** How far occupancy may pass a claimed window before the claim is false.
 *  TWIN: DRIFT_TOLERANCE in modelContextWindows.ts / ContextUsage::reading. */
const DRIFT = 1.02;
/** Context occupancy: the status line's pair unless the held tokens disprove
 *  its window, then the resolved window. Unknown stays unknown. */
export function context(s) {
  const sl = s.statusLine || {}, u = s.usage || {};
  if (sl.contextUsageState === 'waitingForRuntimeUsage') return null;
  const held = typeof u.contextTokens === 'number' ? u.contextTokens : 0;
  const owner = s.resolvedContextWindow || u.contextLimit || null;
  const fromUsage = () => (owner && held > 0 ? { pct: (held / owner) * 100, tokens: held, window: owner } : null);
  const slWindow = sl.contextWindowSize || null;
  if (slWindow && held > slWindow * DRIFT) return fromUsage();
  if (typeof sl.contextUsedPct === 'number' && Number.isFinite(sl.contextUsedPct)) {
    const window = slWindow || owner;
    return { pct: sl.contextUsedPct, tokens: slWindow ? Math.round((sl.contextUsedPct / 100) * slWindow) : held || null, window };
  }
  return fromUsage();
}
export const gaugeTone = (pct) => (pct >= 90 ? 'error' : pct >= 70 ? 'warning' : 'success');
export const costOf = (s) => (s.statusLine && s.statusLine.costUSD) ?? (s.usage && s.usage.costUSD) ?? null;

/** The session's prompt cache once it has expired, judged on this clock;
 *  null while warm, unknown, or working (its next request is on its way). */
export function coldCache(s, now = Date.now()) {
  const c = s && s.promptCache;
  if (!c || typeof c.expiresAt !== 'number' || isWorking(s)) return null;
  return now >= c.expiresAt ? c : null;
}
/** The composer note speaks up from this context size (native LARGE_CONTEXT). */
export const LARGE_CONTEXT = 50000;

/** Background tasks (`background_task_list`), running first then newest. */
export function tasksOf(s) {
  const raw = (s && (s.background_task_list || s.backgroundTaskList)) || [];
  const running = (t) => !['completed', 'failed', 'killed', 'stopped'].includes(t.status);
  const rows = raw.filter((t) => t && t.id).slice(0, 40);
  rows.sort((a, b) => (running(b) - running(a)) ||
    (running(a) ? (a.startedAt || 0) - (b.startedAt || 0) : (b.endedAt || b.startedAt || 0) - (a.endedAt || a.startedAt || 0)));
  return rows.map((t) => ({ ...t, running: running(t) }));
}
export const runningTasks = (s) => tasksOf(s).filter((t) => t.running).length;

/** Provider-native children (Claude Task/Agent subagents), capped like native. */
export function childrenOf(s) {
  const rows = ((s && s.subagents) || []).filter((c) => c && c.id).slice(-32);
  return rows;
}
const CHILD_ACTIVE = new Set(['running', 'responding', 'streaming', 'working', 'thinking', 'background', 'approval', 'question', 'waiting_approval', 'waiting_input']);
export const childActive = (c) => CHILD_ACTIVE.has(c.status || 'running');
export function childStatus(c) {
  const st = c.status || 'running';
  if (['running', 'working', 'thinking', 'responding', 'streaming'].includes(st)) return { label: 'Working', tone: 'busy' };
  if (['complete', 'completed', 'done'].includes(st)) return { label: 'Completed', tone: 'success' };
  if (['stopped', 'ended'].includes(st)) return { label: 'Ended', tone: 'muted' };
  if (['failed', 'error'].includes(st)) return { label: 'Failed', tone: 'error' };
  if (st === 'lost') return { label: 'Unavailable', tone: 'muted' };
  if (['approval', 'waiting_approval'].includes(st)) return { label: 'Needs approval', tone: 'warning' };
  if (['question', 'waiting_input'].includes(st)) return { label: 'Needs input', tone: 'warning' };
  if (['input', 'idle'].includes(st)) return { label: 'Ready', tone: 'success' };
  return { label: 'Starting', tone: 'muted' };
}

// ── cleared children (device-local, like native's cleared_children) ───────
const CLEARED_KEY = 'wks.mnext.clearedChildren';
const cleared = new Set(JSON.parse(localStorage.getItem(CLEARED_KEY) || '[]'));
export const isCleared = (parentId, childId) => cleared.has(parentId + '/' + childId);
export function clearChild(parentId, childId) {
  cleared.add(parentId + '/' + childId);
  localStorage.setItem(CLEARED_KEY, JSON.stringify([...cleared].slice(-500)));
  changed(parentId);
}

// ── shared archive ────────────────────────────────────────────────────────
// Sessions every client of this hub hides from its normal list. View state
// only: archiving never stops or forgets a session.
export let archive = null;          // {version, archived: {id: ms}}
export let archiveReady = false;
let archiveReadSeq = 0, archiveEventSinceRead = false, archiveOpSeq = 0;
const archivePending = new Map();
export function isArchived(id) {
  if (archivePending.has(id)) return archivePending.get(id).on;
  return !!(archive && archive.archived && Object.prototype.hasOwnProperty.call(archive.archived, id));
}
function applyArchive(doc, force) {
  if (!doc || typeof doc.version !== 'number' || !doc.archived || typeof doc.archived !== 'object') return false;
  if (!force && archive && archive.version >= doc.version) return false;
  archive = doc;
  return true;
}
function loadArchive() {
  const seq = ++archiveReadSeq;
  archiveEventSinceRead = false;
  if (!can('sessionArchive.get')) { archiveReady = true; changed(); return; }
  call('sessionArchive.get', {})
    .then((doc) => { if (seq === archiveReadSeq) applyArchive(doc, !archiveEventSinceRead); })
    .catch(() => {})
    .finally(() => { if (seq === archiveReadSeq) { archiveReady = true; changed(); } });
}
/** Archive or restore — visibility only, never a stop. Resolves to an error text or ''. */
export function setArchived(id, on) {
  const op = ++archiveOpSeq;
  archivePending.set(id, { on, op });
  changed();
  const settle = () => { const p = archivePending.get(id); if (p && p.op === op) archivePending.delete(id); };
  return call('sessionArchive.set', { sessionId: id, archived: on })
    .then((doc) => { applyArchive(doc, false); settle(); return ''; })
    .catch((e) => { settle(); return errText(e); })
    .finally(() => changed());
}

// ── config, recents, nodes, usage, jobs ───────────────────────────────────
export let cfg = null;
export let recents = [];
export let nodes = null;
export let usage = null;
export let jobs = null;
export const nodeState = { pending: new Set(), sleeping: new Set(), errors: new Map() };

export async function loadConfig() {
  if (!can('config.get')) return;
  try { cfg = await call('config.get', {}); changed(); } catch { /* derived marks stand */ }
}
export async function loadRecents() {
  if (!can('sessions.recent')) { recents = []; return; }
  try { recents = (await call('sessions.recent', {})) || []; } catch { recents = []; }
  changed();
}
async function loadNodes() {
  // can('nodes.list') is true even on a hub with no registry; the call's
  // "no provider" answer is the feature check, and means absent, not error.
  try { nodes = ((await call('nodes.list', {})) || []).filter((n) => n && n.id); }
  catch { nodes = null; }
  changed();
}
export function applyNodeState(n) {
  if (!n || !n.id) return;
  nodeState.pending.delete(n.id); nodeState.sleeping.delete(n.id); nodeState.errors.delete(n.id);
  if (!nodes) nodes = [n];
  else { const i = nodes.findIndex((x) => x.id === n.id); if (i < 0) nodes.push(n); else nodes[i] = n; }
  changed();
}
let usageAt = 0;
export async function loadUsage(force) {
  if (!can('usage.report') || (!force && Date.now() - usageAt < 60000)) return;
  usageAt = Date.now();
  try { usage = await call('usage.report', {}); } catch { /* keep the last reading */ }
  changed();
}
export async function loadJobs() {
  if (!can('jobs.list')) { jobs = null; return; }
  try { const r = await call('jobs.list', {}); jobs = Array.isArray(r && r.jobs) ? r.jobs : []; }
  catch { jobs = null; }
  changed();
}
export const proposals = () => (jobs || []).filter((j) => j && String(j.proposedBy || '').trim());

// ── conversation (sessions.conversation polling) ──────────────────────────
// Headless snapshots carry no conversation; the transcript lives behind
// sessions.conversation. Items fold into turns here: assistant text from
// stream transports coalesces, PTY blocks dedup, tool results attach to their
// call. `sinceSeq` anchors follow-ups; a backwards seq means the daemon
// rebuilt its log and we resync from zero.
const conv = new Map();
const CONV_REFRESH_MS = 1000;
const MAX_RESULT = 4000;
export function convOf(id) {
  let st = conv.get(id);
  if (!st) { st = { seq: 0, turns: [], tools: new Map(), at: 0, loaded: false, inflight: false, timer: null, error: '' }; conv.set(id, st); }
  return st;
}
/** The turns to render: a rich snapshot's own conversation, else the fetched one. */
export function turnsOf(s) {
  if (s && s.conversation && s.conversation.length) return s.conversation;
  const st = conv.get(s && s.sessionId);
  return (st && st.turns) || [];
}
function pruneConv() {
  for (const [id, st] of conv) if (!sessions.has(id)) { clearTimeout(st.timer); conv.delete(id); }
}
let watching = '';
/** Poll the conversation of the open chat (one at a time). */
export function watchConversation(id) {
  if (watching && watching !== id) { const st = conv.get(watching); if (st) { clearTimeout(st.timer); st.timer = null; } }
  watching = id || '';
  if (id) fetchConv(id);
}
function scheduleConv(id) {
  const st = convOf(id);
  if (st.inflight || st.timer || watching !== id) return;
  const wait = Math.max(0, st.at + CONV_REFRESH_MS - Date.now());
  st.timer = setTimeout(() => { st.timer = null; fetchConv(id); }, wait);
}
async function fetchConv(id) {
  const s = sessions.get(id);
  if (!s) return;
  const st = convOf(id);
  if (st.inflight) return;
  if (s.conversation && s.conversation.length) { st.loaded = true; return; }
  if (hubDown(s)) return;
  st.inflight = true;
  let didChange = false;
  try {
    const params = { sessionId: id };
    if (st.seq > 0) params.sinceSeq = st.seq;
    const r = await call(qualify(id, 'sessions.conversation'), params);
    st.at = Date.now();
    const seq = r && typeof r.seq === 'number' ? r.seq : 0;
    const items = r && Array.isArray(r.items) ? r.items : [];
    if (st.seq > 0 && seq < st.seq) {
      st.seq = 0; st.turns = []; st.tools = new Map(); st.inflight = false;
      return fetchConv(id);
    }
    // Only a real advance counts: an empty log answers {seq:0} forever.
    didChange = items.length > 0 || (st.seq === 0 && seq > 0) || !st.loaded;
    st.seq = seq;
    st.loaded = true;
    st.error = '';
    if (items.length) applyItems(st, items, s);
  } catch (e) { st.at = Date.now(); st.error = errText(e); }
  st.inflight = false;
  if (didChange) changed(id);
  scheduleConv(id);
}
const tsOf = (it) => {
  const raw = it.timestamp || it.updatedAt || it.ts;
  if (typeof raw === 'number') return raw;
  const t = raw ? Date.parse(raw) : NaN;
  return Number.isFinite(t) ? t : 0;
};
export function applyItems(st, items, s) {
  const streaming = !!s && (s.transport === 'stream' || (s.provider && s.provider !== 'claude'));
  const dup = (role, content) => st.turns.slice(-5).some((t) => t.role === role && t.content === content);
  for (const it of items) {
    const kind = it.kind || it.type;
    const at = tsOf(it);
    if (kind === 'user_message') {
      const text = it.text || '';
      if (!text) continue;
      // A stream send is echoed without a timestamp, then the transcript
      // delivers the same turn with one: merge rather than duplicate.
      const echo = st.turns.slice(-5).find((t) => t.role === 'user' && t.content === text && (!t.timestamp || !at));
      if (echo) { if (at) echo.timestamp = at; continue; }
      st.turns.push({ role: 'user', content: text, timestamp: at });
    } else if (kind === 'assistant_text') {
      const text = it.text || '';
      if (!text) continue;
      const last = st.turns[st.turns.length - 1];
      if (streaming && last && last.role === 'assistant' && !(last.toolCalls && last.toolCalls.length)) {
        if (last.content && text.startsWith(last.content)) last.content = text;
        else last.content += text;
        if (at) last.timestamp = at;
      } else if (!dup('assistant', text)) {
        st.turns.push({ role: 'assistant', content: text, timestamp: at });
      }
    } else if (kind === 'tool_use') {
      const tid = it.id || '';
      if (tid && st.tools.has(tid)) continue;
      const tc = { id: tid, name: it.name || 'tool', input: it.input, status: 'running', startedAt: at };
      if (tid) st.tools.set(tid, tc);
      st.turns.push({ role: 'assistant', content: '', timestamp: at, toolCalls: [tc] });
    } else if (kind === 'tool_result') {
      const tc = st.tools.get(it.tool_use_id);
      if (tc) {
        tc.status = it.is_error ? 'failed' : 'complete';
        tc.completedAt = at || tc.startedAt;
        tc.output = String(it.content || '').slice(0, MAX_RESULT);
      }
    } else if (kind === 'slash_command') {
      st.turns.push({ role: 'user', content: '', timestamp: at, command: { name: it.name || '', args: it.args || '' } });
    } else if (kind === 'command_output') {
      for (let i = st.turns.length - 1; i >= 0; i--) {
        const c = st.turns[i].command;
        if (c) { c.output = String(it.output || '').slice(0, MAX_RESULT); c.outputIsError = !!it.is_error; break; }
      }
    } else if (kind === 'plan') {
      st.plan = { steps: Array.isArray(it.steps) ? it.steps : [], at };
    }
  }
  // Bound the client copy: the phone can stay open for days.
  if (st.turns.length > 1500) st.turns.splice(0, st.turns.length - 1500);
}

// ── provider subagent (child) conversation ────────────────────────────────
const childConv = new Map();
export function childConvOf(parentId, agentId) {
  const key = parentId + '/' + agentId;
  let st = childConv.get(key);
  if (!st) { st = { seq: 0, turns: [], tools: new Map(), loaded: false, error: '', at: 0 }; childConv.set(key, st); }
  return st;
}
export async function fetchChild(parentId, agentId) {
  const st = childConvOf(parentId, agentId);
  try {
    const r = await call(qualify(parentId, 'sessions.subagentConversation'), { sessionId: parentId, agentId });
    if (!r) { st.error = 'Child transcript is not available yet'; st.loaded = true; changed(parentId); return; }
    const fresh = { ...st, seq: 0, turns: [], tools: new Map() };
    applyItems(fresh, Array.isArray(r.items) ? r.items : [], { transport: 'stream', provider: 'claude' });
    st.turns = fresh.turns; st.tools = fresh.tools; st.plan = fresh.plan;
    st.loaded = true; st.error = ''; st.at = Date.now();
  } catch (e) { st.loaded = true; st.error = errText(e); }
  changed(parentId);
}

/** Composer drafts survive navigation (and carry a handoff's staged message). */
export const drafts = new Map();

// ── answered questions (client-side trace, anchored by turn count) ────────
export const resolved = new Map();
export function recordResolved(id, declined, answers, questions, anchorLen) {
  if (!questions || !questions.length) return;
  const sig = questions.map((q) => q.question).join('\u0000');
  const list = resolved.get(id) || [];
  if (list.some((r) => r.sig === sig && r.anchorLen === anchorLen)) return;
  list.push({ sig, anchorLen, questions, answers, declined });
  resolved.set(id, list);
}
/** A pick belongs to the question SET it was made against. */
export function questionSig(s) {
  const qs = (s && s.pendingQuestions) || [];
  return hash(qs.map((q) => q.question).join('\u0000'));
}

// ── bus wiring ────────────────────────────────────────────────────────────
onBus('open', () => { seed(); });
onBus('reconcile', () => { seed(); });
onBus('hello', () => {
  loadConfig();
  loadArchive();
  loadRecents();
  loadNodes();
  loadUsage(true);
  loadJobs();
  changed();
});
onBus('state', () => changed());
onBus('event', (ev) => {
  const t = ev.type || '';
  const peer = ev.hub || '';
  if (t === 'agent.snapshot' && ev.data && ev.data.sessionId) {
    const snap = ev.data;
    if (peer) {
      snap.hub = peer;
      sessionHub.set(snap.sessionId, peer);
      // A stamped push can only have crossed a live link.
      offlineHubs.delete(peer);
    }
    upsert(snap);
    if (watching === snap.sessionId) scheduleConv(snap.sessionId);
  } else if (t === 'hub.peer.disconnected') {
    const d = ev.data || {};
    if (d.peer) { offlineHubs.set(d.peer, Date.parse(d.lastSeen || '') || Date.now()); changed(); }
  } else if (t === 'hub.peer.connected') {
    const d = ev.data || {};
    if (d.peer) { offlineHubs.delete(d.peer); reseedPeer(d.peer); }
  } else if (t === 'node.state_changed') {
    applyNodeState((ev.data || {}).node);
  } else if (t === 'sessionArchive.changed' && !peer) {
    archiveEventSinceRead = true;
    if (applyArchive(ev.data, false)) changed();
  } else if (t.indexOf('workflow.') === 0) {
    changed();
  }
});

export { bus };
