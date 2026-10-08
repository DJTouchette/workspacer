// Fleet/supervisor wakes and workers' structured results.
//
// [fleet]/[supervisor] wakes reach a manager's conversation through the
// plain-text message endpoint, so they land as an ordinary user turn and are
// recognized from the text alone. The grammar is pinned by
// contracts/fleet-message-cases.json (main/shared/fleetMessages.ts is the
// writer); this parser is ported verbatim from /m and must stay byte-for-byte
// compatible with it. Rendering follows native's fleet_card.rs.
import { esc, ic, oneLine } from './util.js';

const FLEET_HEADERS = {
  'worker-finished': '[fleet] Worker finished:',
  'worker-escalated': '[fleet] Worker escalated — blocked and did not complete:',
  'catch-up': '[fleet] Catch-up — these workers finished while you were idle and you may have missed the wake:',
  blocked: '[supervisor] An agent is now blocked on a decision:',
  threshold: '[fleet] A threshold you asked to be told about has been crossed:',
  progress: '[fleet] Progress update from a worker — it is STILL RUNNING; this is NOT a completion:',
};
const FLEET_ALT_HEADERS = { 'worker-finished': '[fleet] Worker FAILED — did not complete:' };
const FLEET_ENTRY_RE =
  /^(.+?) \(session:([\w-]+), (?:cwd (.+?)|(approval|question))\)(?: — (stopped\/killed))?(?: — FAILED: ((?:(?! — ).)+))?(?: — crossed: ((?:(?! — ).)+))?(?: — (NEEDS A DECISION))?(?: — (?:last reply: (.*)|reports: (.*)))?$/;

function parseFleetEntry(body) {
  const m = FLEET_ENTRY_RE.exec(body);
  if (!m) return null;
  const [, label, sessionId, cwd, blockedOn, stopped, failed, crossed, decision, lastReply, note] = m;
  const e = { label, sessionId };
  if (cwd !== undefined) e.cwd = cwd;
  if (blockedOn) e.blockedOn = blockedOn;
  if (stopped) e.stopped = true;
  if (failed) e.failed = failed;
  if (crossed) e.crossed = crossed;
  if (decision) e.needsDecision = true;
  if (lastReply) e.lastReply = lastReply;
  if (note) e.note = note;
  return e;
}

const FLEET_RESULT_RE = /^Structured result — .+? \(session:([\w-]+)\):\n([\s\S]+)$/;
const FLEET_RESULT_MISSING_RE =
  /^Structured result MISSING — .+? \(session:([\w-]+)\): ([\s\S]+?)\. Read the prose report below\/above instead\.$/;
const FLEET_ESCALATION_RE = /^Worker escalation — .+? \(session:([\w-]+)\):\n([\s\S]+)$/;
const FLEET_ESCALATION_INVALID_RE =
  /^Worker escalation INVALID — .+? \(session:([\w-]+)\): ([\s\S]+?)\. The terminal marker was rejected; treat the prose as an ordinary completion or refusal\.$/;
// Every result block precedes the first full-reply block, and a full reply is
// arbitrary worker prose: stop there so its paragraphs cannot forge a result.
const FLEET_FULL_REPLY_MARK = 'Full final message — ';

function attachFleetResultBlocks(tailLines, entries) {
  const byId = new Map(entries.map((e) => [e.sessionId, e]));
  for (const raw of tailLines.join('\n').split('\n\n')) {
    const block = raw.trim();
    if (block.startsWith(FLEET_FULL_REPLY_MARK)) return;
    const ok = FLEET_RESULT_RE.exec(block);
    if (ok) { const entry = byId.get(ok[1]); if (entry) entry.result = ok[2]; continue; }
    const missing = FLEET_RESULT_MISSING_RE.exec(block);
    if (missing) { const entry = byId.get(missing[1]); if (entry) entry.resultError = missing[2]; continue; }
    const escalation = FLEET_ESCALATION_RE.exec(block);
    if (escalation) { const entry = byId.get(escalation[1]); if (entry) entry.escalation = escalation[2]; continue; }
    const invalidEscalation = FLEET_ESCALATION_INVALID_RE.exec(block);
    if (invalidEscalation) { const entry = byId.get(invalidEscalation[1]); if (entry) entry.escalationError = invalidEscalation[2]; }
  }
}

/** Recognize an injected wake; null unless EVERY bullet parses. */
export function parseFleetMessage(text) {
  const trimmed = String(text || '').trim();
  for (const kind of Object.keys(FLEET_HEADERS)) {
    const header = [FLEET_HEADERS[kind], FLEET_ALT_HEADERS[kind]].find((h) => h && trimmed.startsWith(h));
    if (!header) continue;
    const rest = trimmed.slice(header.length);
    if (!rest.startsWith('\n')) return null;
    const lines = rest.split('\n').slice(1);
    const entries = [];
    let i = 0;
    for (; i < lines.length; i++) {
      if (!lines[i].startsWith('- ')) break;
      const e = parseFleetEntry(lines[i].slice(2));
      if (!e) return null;
      entries.push(e);
    }
    if (entries.length === 0) return null;
    attachFleetResultBlocks(lines.slice(i), entries);
    return { kind, entries };
  }
  return null;
}

// ── a worker's structured result (structuredResultFields.ts) ──────────────
// Fields are classified by VALUE SHAPE, not key: a boolean is a badge, a
// number a counter, a SHA-shaped string a commit, path-shaped strings a file
// list, records key/value rows. Empty values say so; no field is dropped.
const RESULT_KNOWN_ORDER = ['merged', 'commit', 'caveats', 'filesChanged', 'checksRun', 'followUps'];
const RESULT_CAVEAT_KEYS = new Set(['caveat', 'caveats']);
const RESULT_SHA_RE = /^[0-9a-f]{7,40}$/i;
const RESULT_COMMIT_KEY_RE = /(^|[^a-z])(commit|sha|revision|rev)s?$/i;
const RESULT_PATH_RE = /^[^\s:]*[/\\][^\s]*$/;
const TEXT_CLAMP = 280, CAVEATS_CLAMP = 700;

function humanizeKey(key) {
  const words = key.replace(/[_-]+/g, ' ').replace(/([a-z0-9])([A-Z])/g, '$1 $2')
    .replace(/([A-Z]+)([A-Z][a-z])/g, '$1 $2').trim();
  return words ? words.toLowerCase() : key;
}
function looksLikeCommit(key, value) {
  if (typeof value !== 'string') return false;
  const v = value.trim();
  if (!v) return false;
  if (RESULT_COMMIT_KEY_RE.test(key)) return v.length <= 64 && !/\s/.test(v);
  return RESULT_SHA_RE.test(v);
}
const isObj = (v) => typeof v === 'object' && v !== null && !Array.isArray(v);
const isScalar = (v) => typeof v === 'string' || typeof v === 'number' || typeof v === 'boolean';
function classify(key, value) {
  if (value === null || value === undefined) return 'empty';
  if (typeof value === 'boolean') return 'boolean';
  if (typeof value === 'number') return Number.isFinite(value) ? 'number' : 'empty';
  if (typeof value === 'string') return !value.trim() ? 'empty' : looksLikeCommit(key, value) ? 'commit' : 'text';
  if (Array.isArray(value)) {
    if (!value.length) return 'empty';
    if (!value.every(isScalar)) return 'list';
    return value.every((v) => typeof v === 'string' && RESULT_PATH_RE.test(v)) ? 'paths' : 'strings';
  }
  if (isObj(value)) return Object.keys(value).length ? 'object' : 'empty';
  return 'empty';
}
function slotFor(key, kind) {
  if (RESULT_CAVEAT_KEYS.has(key.toLowerCase())) return 'caveats';
  return kind === 'boolean' || kind === 'number' || kind === 'commit' ? 'summary' : 'body';
}
const describe = (key, value) => { const kind = classify(key, value); return { key, label: humanizeKey(key), kind, value, slot: slotFor(key, kind) }; };
function orderKeys(keys) {
  const rank = (k) => { const i = RESULT_KNOWN_ORDER.indexOf(k); return i < 0 ? RESULT_KNOWN_ORDER.length : i; };
  return keys.map((k, i) => ({ k, i })).sort((a, b) => rank(a.k) - rank(b.k) || a.i - b.i).map((x) => x.k);
}
/** Never throws: unparseable JSON comes back as a fallback with its reason. */
export function buildResultView(json) {
  const text = String(json ?? '').trim();
  if (!text) return { fields: [], fallback: { text: '', reason: 'the result block was empty' } };
  let parsed;
  try { parsed = JSON.parse(text); } catch {
    return { fields: [], fallback: { text, reason: /\[truncated:/.test(text) ? 'the result was too large for the wake and arrived truncated' : 'the result is not valid JSON' } };
  }
  if (!isObj(parsed)) return { fields: [describe('result', parsed)] };
  return { fields: orderKeys(Object.keys(parsed)).map((k) => describe(k, parsed[k])) };
}
const itemText = (item) => typeof item === 'string' ? item : isScalar(item) ? String(item) :
  item == null ? '—' : (() => { try { return JSON.stringify(item); } catch { return String(item); } })();
const clampText = (t, n) => { t = String(t ?? ''); return t.length > n ? t.slice(0, n).trimEnd() + '…' : t; };
const emptyLabel = (v) => Array.isArray(v) ? 'none' : v == null ? 'not reported' : typeof v === 'string' ? 'empty' : 'none';

function chipHtml(f) {
  if (f.kind === 'boolean') {
    const yes = f.value === true;
    return `<span class="badge ${yes ? 'success' : 'neutral'}">${ic(yes ? 'check' : 'x', 's12')}${esc(f.label)}</span>`;
  }
  if (f.kind === 'number') return `<span class="badge neutral"><b>${esc(Number.isInteger(f.value) ? f.value.toLocaleString('en-US') : String(f.value))}</b> ${esc(f.label)}</span>`;
  const full = String(f.value).trim();
  const short = RESULT_SHA_RE.test(full) && full.length > 12 ? full.slice(0, 8) : full;
  return `<button class="badge neutral mono" data-copy="${esc(full)}">${ic('git-branch', 's12')}${esc(short)}${f.key !== 'commit' ? ' ' + esc(f.label) : ''}</button>`;
}
function bodyFieldHtml(f, openKey, open) {
  let inner;
  if (f.kind === 'empty') inner = `<span class="faint">${esc(emptyLabel(f.value))}</span>`;
  else if (f.kind === 'text') inner = `<span>${esc(clampText(f.value, TEXT_CLAMP))}</span>`;
  else if (f.kind === 'paths') {
    const key = `${openKey}:p:${f.key}`;
    inner = `<button class="disc" data-toggle="${esc(key)}">${ic(open.has(key) ? 'chevron-down' : 'chevron-right', 's12')}<b>${f.value.length}</b> ${esc(f.label)}</button>` +
      (open.has(key) ? `<div class="paths">${f.value.map((p) => `<div class="mono">${esc(p)}</div>`).join('')}</div>` : '');
    return `<div class="rf">${inner}</div>`;
  } else if (f.kind === 'strings' || f.kind === 'list') {
    const key = `${openKey}:l:${f.key}`;
    const long = f.value.length >= 5 && !open.has(key);
    const shown = long ? f.value.slice(0, 3) : f.value;
    inner = `<ul>${shown.map((v) => `<li>${esc(clampText(itemText(v), TEXT_CLAMP))}</li>`).join('')}</ul>` +
      (f.value.length >= 5 ? `<button class="more" data-toggle="${esc(key)}">${long ? `+${f.value.length - 3} more` : 'Show fewer'}</button>` : '');
  } else {
    inner = Object.entries(f.value).map(([k, v]) => `<div class="kv"><span class="muted">${esc(k)}</span><span>${esc(clampText(itemText(v), TEXT_CLAMP))}</span></div>`).join('');
  }
  return `<div class="rf"><div class="overline">${esc(f.label)}</div>${inner}</div>`;
}

/** Native's "Structured result" card. `escalation` renders the worker's
 *  terminal escalation block instead. `openKey` scopes disclosures. */
export function resultCardHtml(json, error, { title = 'Structured result', openKey = '', open = new Set(), escalation = false } = {}) {
  if (!json && !error) return '';
  const view = json ? buildResultView(json) : { fields: [] };
  const summary = view.fields.filter((f) => f.slot === 'summary');
  const caveats = view.fields.filter((f) => f.slot === 'caveats');
  const body = view.fields.filter((f) => f.slot === 'body');
  const key = `${openKey}:${escalation ? 'escalation' : 'result'}`;
  let html = `<div class="result${escalation ? ' escalation' : ''}"><div class="rh">${ic(escalation ? 'triangle-alert' : 'list-checks', 's14')}<b>${esc(title)}</b></div>` +
    `<div class="rsub">Worker-reported · checks and outcomes have no host verification</div>`;
  if (error) html += `<div class="rwarn">${ic('triangle-alert', 's14')}<span>${escalation ? 'Invalid worker escalation' : 'No structured result'} — ${esc(error)}</span></div>`;
  if (view.fallback) {
    html += `<div class="rwarn">${ic('triangle-alert', 's14')}<span>${esc(view.fallback.reason)} — shown as it arrived</span></div>`;
    if (view.fallback.text) html += `<pre class="raw">${esc(view.fallback.text)}</pre>`;
  }
  if (summary.length) html += `<div class="chips">${summary.map(chipHtml).join('')}</div>`;
  for (const f of caveats) {
    html += `<div class="caveats"><div class="overline">${ic('triangle-alert', 's12')}${esc(f.label)}</div><div>${esc(
      f.kind === 'empty' ? (f.value == null ? 'not reported' : 'none reported') : clampText(itemText(f.value), CAVEATS_CLAMP))}</div></div>`;
  }
  if (body.length) html += `<div class="rbody">${body.map((f) => bodyFieldHtml(f, key, open)).join('')}</div>`;
  return html + '</div>';
}

// ── the wake card (native fleet_card.rs) ──────────────────────────────────
const KIND = {
  'worker-finished': { title: ['Fleet · worker finished', 'Fleet · workers finished'], tone: 'success', icon: 'circle-check' },
  'worker-escalated': { title: ['Fleet · worker escalated', 'Fleet · workers escalated'], tone: 'warning', icon: 'triangle-alert' },
  'catch-up': { title: ['Fleet · catch-up', 'Fleet · catch-up'], tone: 'success', icon: 'history' },
  threshold: { title: ['Fleet · threshold crossed', 'Fleet · thresholds crossed'], tone: 'warning', icon: 'triangle-alert' },
  progress: { title: ['Fleet · progress update', 'Fleet · progress updates'], tone: 'accent', icon: 'megaphone' },
  blocked: { title: ['Supervisor · decision needed', 'Supervisor · decisions needed'], tone: 'warning', icon: 'triangle-alert' },
};
const FAILED = { title: ['Fleet · worker failed', 'Fleet · workers failed'], tone: 'error', icon: 'circle-x' };

export function fleetCardHtml(msg, { openKey = '', open = new Set(), known = () => false } = {}) {
  const allFailed = msg.kind === 'worker-finished' && msg.entries.every((e) => e.failed);
  const meta = allFailed ? FAILED : KIND[msg.kind];
  const plural = msg.entries.length > 1;
  const rows = msg.entries.map((e) => {
    const badges = [];
    if (e.stopped) badges.push('<span class="badge error">Stopped</span>');
    if (e.failed) badges.push(`<span class="badge error" title="${esc(e.failed)}">Failed</span>`);
    else if (msg.kind === 'worker-finished' || msg.kind === 'catch-up') badges.push('<span class="badge success">Finished</span>');
    if (e.crossed) badges.push(`<span class="badge warning">${esc(oneLine(e.crossed, 40))}</span>`);
    if (e.needsDecision) badges.push('<span class="badge warning">Needs a decision</span>');
    if (e.blockedOn) badges.push(`<span class="badge warning">${e.blockedOn === 'approval' ? 'Needs approval' : 'Needs your input'}</span>`);
    const id = e.sessionId.length > 10 ? e.sessionId.slice(0, 8) + '…' : e.sessionId;
    const replyKey = `${openKey}:${e.sessionId}:reply`;
    const detail = [
      e.failed ? `<div class="fr-note t-error">${esc(e.failed)}</div>` : '',
      e.note ? `<div class="fr-note">${esc(e.note)}</div>` : '',
      resultCardHtml(e.result, e.resultError, { openKey: `${openKey}:${e.sessionId}`, open }),
      resultCardHtml(e.escalation, e.escalationError, { title: 'Worker escalation', openKey: `${openKey}:${e.sessionId}`, open, escalation: true }),
      e.lastReply ? `<button class="disc" data-toggle="${esc(replyKey)}">${ic(open.has(replyKey) ? 'chevron-down' : 'chevron-right', 's12')}Last reply</button>` +
        (open.has(replyKey) ? `<div class="fr-note">${esc(e.lastReply)}</div>` : '') : '',
    ].join('');
    return `<div class="fr">
      <div class="fr-a"><span class="fr-nm">${esc(e.label)}</span>${badges.join('')}
        <span class="acts">${known(e.sessionId) ? `<button class="ibtn sm" data-open="${esc(e.sessionId)}" aria-label="Open ${esc(e.label)}">${ic('arrow-right', 's16')}</button>` : ''}
        <button class="ibtn sm" data-mention="${esc(e.sessionId)}" aria-label="Reply about ${esc(e.label)}">${ic('reply', 's16')}</button></span></div>
      <div class="fr-m mono">session:${esc(id)}${e.cwd && e.cwd !== '?' ? ` · ${esc(e.cwd.replace(/[/\\]+$/, '').split(/[/\\]/).pop())}` : ''}</div>
      ${detail}
    </div>`;
  }).join('');
  return `<div class="fleet" style="--tone:var(--wks-${meta.tone})">
    <div class="fh">${ic(meta.icon, `s16 t-${meta.tone}`)}<span class="overline" style="color:var(--wks-${meta.tone})">${esc(meta.title[plural ? 1 : 0])}${plural ? ` · ${msg.entries.length}` : ''}</span></div>
    ${rows}
  </div>`;
}
