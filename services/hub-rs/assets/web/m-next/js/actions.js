// Everything the phone asks a session to do. Each call is hub-qualified for a
// federated session and refused locally when that hub's link is down. Results
// come back as a notice; the snapshot that follows is the real confirmation.
import { call, can, bus } from './bus.js';
import { notice } from './ui.js';
import {
  sessions, qualify, hubGone, turnsOf, recordResolved, seed, changed, providerOf, modelOf, effortOf,
  permissionOf, isPaused,
} from './store.js';
import { errText, shortModel } from './util.js';

const offline = (id) => { if (hubGone(id)) { notice('That hub is offline', 'warning'); return true; } return false; };

export async function approve(id, decision) {
  if (offline(id)) return false;
  try {
    await call(qualify(id, 'claude.approve'), { sessionId: id, decision });
    notice(decision === 'no' ? 'Denied' : 'Allowed once', 'success');
    return true;
  } catch (e) { notice('Could not answer: ' + errText(e), 'error'); return false; }
}

/** answers: 1-based option picks as strings, or free text per question. */
export async function answer(id, answers, labels) {
  if (offline(id)) return false;
  const s = sessions.get(id);
  const questions = (s && s.pendingQuestions) || [];
  const anchorLen = s ? turnsOf(s).length : 0;
  try {
    await call(qualify(id, 'claude.answer'), { sessionId: id, answers });
    recordResolved(id, false, labels, questions, anchorLen);
    notice('Answer sent', 'success');
    changed(id);
    return true;
  } catch (e) { notice('Could not send the answer: ' + errText(e), 'error'); return false; }
}

export async function declineQuestion(id) {
  if (offline(id)) return;
  const s = sessions.get(id);
  recordResolved(id, true, null, (s && s.pendingQuestions) || [], s ? turnsOf(s).length : 0);
  try { await call(qualify(id, 'claude.signal'), { sessionId: id, signal: 'SIGINT' }); notice('Declined'); }
  catch (e) { notice('Could not decline: ' + errText(e), 'error'); }
  changed(id);
}

export async function interrupt(id) {
  if (offline(id)) return;
  try { await call(qualify(id, 'claude.signal'), { sessionId: id, signal: 'SIGINT' }); notice('Interrupted'); }
  catch (e) { notice('Could not interrupt: ' + errText(e), 'error'); }
}

/** SIGTERM: ends the process; the session stays and can be resumed. */
export async function endSession(id) {
  if (offline(id)) return;
  try { await call(qualify(id, 'claude.signal'), { sessionId: id, signal: 'SIGTERM' }); notice('Session ended'); }
  catch (e) { notice('Could not end the session: ' + errText(e), 'error'); }
}

/** Send to a live session. Resolves true when the hub took it. */
export async function send(id, text) {
  if (offline(id)) return false;
  try {
    await call(qualify(id, 'agents.sendMessage'), { sessionId: id, text });
    return true;
  } catch (e) { notice('Not sent: ' + errText(e), 'error'); return false; }
}

/** Native's permission vocabulary on the spawn wire. */
export function accessWire(provider, mode) {
  const claude = provider === 'claude';
  if (mode === 'bypassPermissions' || mode === 'yolo' || mode === 'full') return claude ? 'bypassPermissions' : 'yolo';
  if (claude && (mode === 'acceptEdits' || mode === 'plan')) return mode;
  return claude ? 'default' : 'ask';
}
export const fullAccess = (mode) => mode === 'bypassPermissions' || mode === 'yolo';

/** Resume a stopped session with `text` as its next message: the same
 *  session (same id and conversation) on its provider, folder, model, effort
 *  and access — native kept.rs `resume_with`. */
export async function resumeWith(id, text) {
  const s = sessions.get(id);
  if (!s || offline(id)) return false;
  if (!can('agents.spawn')) { notice('Resuming needs an operator token', 'warning'); return false; }
  const provider = providerOf(s) === 'codex' ? 'codex' : 'claude';
  const mode = accessWire(provider, permissionOf(s));
  if (fullAccess(mode) && !bus.scope.fullAccess) {
    notice('This session had full access, which this device cannot grant. Resume it on the desktop.', 'warning');
    return false;
  }
  const params = { provider, cwd: s.cwd, transport: 'stream', resumeSessionId: id, permissionMode: mode, skipPermissions: fullAccess(mode) };
  const model = modelOf(s);
  if (model) params.model = model;
  const effort = effortOf(s);
  if (effort) params.effort = effort;
  if (s.label) params.label = s.label;
  if (text) params.message = text;
  try {
    const r = await call(qualify(id, 'agents.spawn'), params, 60000);
    if (text && !(r && r.messageQueued) && r && r.sessionId) {
      try { await call(qualify(r.sessionId, 'agents.sendMessage'), { sessionId: r.sessionId, text }); }
      catch (e) { notice('Resumed, but the message was not sent: ' + errText(e), 'warning'); }
    }
    notice(isPaused(s) ? 'Picking up where it left off' : 'Resumed', 'success');
    await seed();
    return (r && r.sessionId) || id;
  } catch (e) { notice('Could not resume this session: ' + errText(e), 'error'); return false; }
}

// ── live switches (model / effort / access) ──────────────────────────────
let switchUnsupported = false;
export const canSwitch = () => !switchUnsupported && can('claude.setModel');
function switchFailed(e) {
  const m = errText(e);
  if (/no provider|unknown method|not authorized/i.test(m)) switchUnsupported = true;
  notice('Not changed: ' + m, 'error');
}
export async function setModel(id, model, identity, contextWindow) {
  const params = { sessionId: id, model };
  if (identity) params.modelIdentity = identity;
  if (contextWindow != null) params.contextWindow = contextWindow;
  try {
    const r = await call(qualify(id, 'claude.setModel'), params);
    if (r && r.ok === false) { notice(r.error || 'Model change refused', 'error'); return; }
    const queued = r && (r.queued === true || r.disposition === 'queued');
    const label = shortModel((r && (r.model || (r.requestedSelection && r.requestedSelection.model))) || model);
    notice(queued ? `Switches to ${label} when this turn ends` : `Model: ${label}`, 'success');
  } catch (e) { switchFailed(e); }
}
export async function setEffort(id, effort) {
  const s = sessions.get(id);
  try {
    const r = providerOf(s) === 'claude'
      ? await call(qualify(id, 'claude.setEffort'), { sessionId: id, effort })
      : await call(qualify(id, 'claude.setModel'), { sessionId: id, effort });
    if (r && r.ok === false) { notice(r.error || 'Effort change refused', 'error'); return; }
    notice('Effort: ' + effort, 'success');
  } catch (e) { switchFailed(e); }
}
export async function setAccess(id, mode) {
  try {
    const r = await call(qualify(id, 'claude.setPermissionMode'), { sessionId: id, mode });
    if (r && r.ok === false) { notice(r.error || 'Access change refused', 'error'); return; }
    notice('Access: ' + accessLabel(mode), 'success');
  } catch (e) { switchFailed(e); }
}

export const CLAUDE_MODES = [
  { id: 'default', label: 'Ask to approve', detail: 'The agent asks when a tool needs approval.' },
  { id: 'acceptEdits', label: 'Accept edits', detail: 'Allow file edits; other tools may still need approval.' },
  { id: 'plan', label: 'Plan mode', detail: 'Explore and plan before making changes.' },
  { id: 'bypassPermissions', label: 'Full access', detail: 'Run tools without approval prompts.' },
];
export const MANAGED_MODES = [
  { id: 'ask', label: 'Ask to approve', detail: 'The agent asks when a tool needs approval.' },
  { id: 'yolo', label: 'Full access', detail: 'Run tools without approval prompts.' },
];
export const modesFor = (provider) => (provider === 'claude' ? CLAUDE_MODES : MANAGED_MODES);
export const accessLabel = (mode) =>
  [...CLAUDE_MODES, ...MANAGED_MODES].find((m) => m.id === mode)?.label || ({ auto: 'Auto', dontAsk: "Don't ask" })[mode] || 'Ask to approve';
export const effortsFor = (provider) =>
  provider === 'claude' ? ['low', 'medium', 'high', 'xhigh', 'max'] : provider === 'codex' ? ['low', 'medium', 'high', 'xhigh'] : [];

/** Pair-aware Claude catalog value (legacy `[1m]` marker ↔ canonical id + window). */
export function claudeModelWire(model, contextWindow) {
  let identity = typeof model === 'string' ? model.trim() : '';
  if (!identity) return null;
  let markerWindow = null;
  while (/(?:\[1m\]|-1m)$/i.test(identity)) { identity = identity.replace(/(?:\[1m\]|-1m)$/i, '').trimEnd(); markerWindow = 1000000; }
  if (!identity || (contextWindow != null && contextWindow <= 0)) return null;
  if (markerWindow != null && contextWindow != null && markerWindow !== contextWindow) return null;
  const window = contextWindow == null ? markerWindow : contextWindow;
  const native1m = /(?:fable|mythos)/i.test(identity);
  return { id: identity, legacy: window === 1000000 && !native1m ? identity + '[1m]' : identity, contextWindow: window };
}

/** A provider's model catalog as [{id, legacy, contextWindow, label}]. */
export async function listModels(provider, cwd, sessionId) {
  const method = (m) => (sessionId ? qualify(sessionId, m) : m);
  if (provider === 'claude') {
    const r = await call(method('claude.listModels'), {});
    const rows = [
      ...((r && r.aliases) || []).map((a) => ({ value: a.model || a.value, cw: a.contextWindow ?? null, legacy: a.value, label: a.label })),
      ...((r && r.seen) || []).map((value) => ({ value, cw: null })),
    ];
    const seen = new Set(), labels = new Set(), out = [];
    for (const m of rows) {
      let sel = claudeModelWire(m.value, m.cw);
      if (sel && sel.contextWindow == null && m.legacy) {
        const compat = claudeModelWire(m.legacy, null);
        if (compat && compat.id.toLowerCase() === sel.id.toLowerCase()) sel = compat;
      }
      if (!sel) continue;
      const key = sel.id + '\u0000' + sel.contextWindow;
      const label = m.label || shortModel(sel.legacy) || sel.id;
      if (seen.has(key) || labels.has(label)) continue;
      seen.add(key); labels.add(label);
      out.push({ ...sel, label });
    }
    return out;
  }
  const list = await call(method('providers.listModels'), { provider, cwd });
  return (Array.isArray(list) ? list : []).filter((m) => m && typeof m.id === 'string')
    .map((m) => ({ id: m.id, legacy: m.id, contextWindow: m.contextWindow ?? null, label: m.label || m.id }));
}

// ── handoff: "Continue with…" and "Start fresh from a summary" ────────────
/** The takeover message staged in the successor's composer (native/desktop wording). */
export const successorPrompt = (path) =>
  `You are taking over an in-progress session from another AI coding agent. First read the handoff brief at ${path}, ` +
  "then continue the work from where it left off — don't start over or redo completed steps. Reply with a one-paragraph " +
  'summary of the state and your next step.';
export const BRIEF_METHOD = { agent: 'claude.handoffAgentBrief', summary: 'claude.handoffSummaryBrief', mechanical: 'claude.handoffBrief' };

/** Write a brief on the source's hub, then start the successor in the
 *  source's exact folder with no message sent. Resolves {sessionId, prompt}
 *  or null; reports honestly about fallbacks and orphaned briefs. */
export async function handoff(sourceId, { provider, brief, model, contextWindow, effort, mode }) {
  const s = sessions.get(sourceId);
  if (!s || offline(sourceId)) return null;
  let reply;
  try { reply = await call(qualify(sourceId, BRIEF_METHOD[brief]), { sessionId: sourceId }, 180000); }
  catch (e) { notice('Could not prepare the handoff brief: ' + errText(e), 'error'); return null; }
  if (!reply || reply.ok === false || !reply.path) {
    notice('Could not prepare the handoff brief: ' + ((reply && reply.error) || 'the hub wrote no brief'), 'error');
    return null;
  }
  const wire = accessWire(provider, mode);
  const params = { provider, cwd: s.cwd, transport: 'stream', permissionMode: wire, skipPermissions: fullAccess(wire), autoTitle: true };
  if (model) params.model = model;
  if (contextWindow != null) params.contextWindow = contextWindow;
  if (effort) params.effort = effort;
  try {
    const r = await call(qualify(sourceId, 'agents.spawn'), params, 60000);
    const id = r && r.sessionId;
    if (!id) throw new Error('the hub returned no session');
    const who = brief === 'agent' ? 'The source agent' : brief === 'summary' ? 'The summary model' : 'The hub';
    notice(reply.fallback
      ? `Handoff ready with a fallback brief: ${who} did not write it, so the hub's summary was used.`
      : 'Handoff ready: review the message, then send it.', reply.fallback ? 'warning' : 'success');
    await seed();
    return { sessionId: id, prompt: successorPrompt(reply.path) };
  } catch (e) {
    notice(`The brief was written to ${reply.path}, but the new agent could not start: ${errText(e)}`, 'error');
    return null;
  }
}

// ── background tasks ─────────────────────────────────────────────────────
export async function taskOutput(sessionId, taskId, offset) {
  const params = { sessionId, taskId, maxBytes: 256 * 1024 };
  if (offset != null) params.offset = offset;
  const r = await call(qualify(sessionId, 'sessions.taskOutput'), params);
  if (!r || r.task_id !== taskId) throw new Error('The log belongs to another task');
  return r;
}
export async function stopTask(sessionId, taskId) {
  try { await call(qualify(sessionId, 'sessions.taskStop'), { sessionId, taskId }); notice('Stop requested'); return true; }
  catch (e) { notice('Could not stop the task: ' + errText(e), 'error'); return false; }
}

// ── attachments ──────────────────────────────────────────────────────────
const UPLOAD_MAX_EDGE = 2048, PASSTHRU_BYTES = 2 * 1024 * 1024;
/** Decode via <img> (Safari decodes HEIC), bound the long edge, re-encode
 *  JPEG; small PNG/JPEG pass through so screenshots keep crisp text. */
export function normalizeImage(file) {
  return new Promise((resolve, reject) => {
    const passthru = (file.type === 'image/png' || file.type === 'image/jpeg') && file.size <= PASSTHRU_BYTES;
    const url = URL.createObjectURL(file);
    const img = new Image();
    img.onload = () => {
      const draw = (edge, type, q) => {
        const k = Math.min(1, edge / Math.max(img.naturalWidth, img.naturalHeight));
        const c = document.createElement('canvas');
        c.width = Math.max(1, Math.round(img.naturalWidth * k));
        c.height = Math.max(1, Math.round(img.naturalHeight * k));
        c.getContext('2d').drawImage(img, 0, 0, c.width, c.height);
        return c.toDataURL(type, q);
      };
      const thumb = draw(88, 'image/jpeg', 0.7);
      if (passthru) {
        const r = new FileReader();
        r.onload = () => { URL.revokeObjectURL(url); resolve({ name: file.name, dataUrl: r.result, thumb }); };
        r.onerror = () => { URL.revokeObjectURL(url); reject(new Error('read failed')); };
        r.readAsDataURL(file);
        return;
      }
      const dataUrl = draw(UPLOAD_MAX_EDGE, 'image/jpeg', 0.85);
      URL.revokeObjectURL(url);
      resolve({ name: ((file.name || 'photo').replace(/\.[^.]*$/, '') || 'photo') + '.jpg', dataUrl, thumb });
    };
    img.onerror = () => { URL.revokeObjectURL(url); reject(new Error("couldn't decode " + (file.name || 'image'))); };
    img.src = url;
  });
}
/** Upload to the machine RUNNING the agent (files.upload federates). */
export async function upload(sessionId, name, dataUrl) {
  const res = await call(qualify(sessionId, 'files.upload'), { name, dataBase64: dataUrl.slice(dataUrl.indexOf(',') + 1) }, 90000);
  if (!res || !res.path) throw new Error('no path returned');
  return res.path;
}
