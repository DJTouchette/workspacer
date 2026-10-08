// A conversation as native's transcript draws it: "You" bubbles, assistant
// prose (markdown, response cards, worker results), a work row per run of
// tool calls (expandable), spawn-agent cards with their live child, the
// files a turn changed, and "Took 1m 12s" under each finished turn.
import { esc, ic, mark, spinner, fmtElapsed, when, oneLine, basename, modelName } from '../util.js';
import { renderMarkdown, assistantBlocks } from '../markdown.js';
import { parseCard, cardHtml } from '../card.js';
import { parseFleetMessage, fleetCardHtml, resultCardHtml } from '../fleet.js';
import { sessions, childrenOf, childStatus, childActive, providerOf, titleOf, status } from '../store.js';

const EDIT = new Set(['edit', 'multiedit', 'write', 'notebookedit', 'apply_patch', 'patch', 'create_file', 'str_replace_based_edit_tool']);
const READ = new Set(['read', 'read_file', 'view']);
const SHELL = new Set(['bash', 'shell', 'exec_command', 'run_command', 'terminal', 'local_shell']);
const SEARCH = new Set(['grep', 'glob', 'search', 'find', 'ls']);
const SPAWN = new Set(['agent', 'task']);

const lower = (n) => String(n || '').toLowerCase().replace(/^mcp__[^_]+__/, '');
const isSpawn = (tc) => SPAWN.has(lower(tc.name)) || /spawn_agent$/.test(String(tc.name || ''));
function firstString(input) {
  if (!input || typeof input !== 'object') return typeof input === 'string' ? input : '';
  for (const v of Object.values(input)) if (typeof v === 'string' && v) return v;
  return '';
}
/** The icon, verb and target a tool row shows (native transcript.rs names). */
export function toolInfo(tc) {
  const n = lower(tc.name), i = tc.input || {};
  const file = basename(i.file_path || i.path || i.filePath || i.notebook_path || '');
  if (EDIT.has(n)) return { icon: 'pencil', verb: n === 'write' || n === 'create_file' ? 'Write' : 'Edit', arg: file || (Array.isArray(i.changes) ? `${i.changes.length} files` : 'patch') };
  if (READ.has(n)) return { icon: 'file', verb: 'Read', arg: file };
  if (SHELL.has(n)) {
    const raw = i.command ?? i.cmd;
    return { icon: 'square-terminal', verb: 'Command', arg: oneLine(Array.isArray(raw) ? raw.join(' ') : String(raw ?? ''), 120) };
  }
  if (SEARCH.has(n)) return { icon: 'search', verb: n === 'glob' ? 'Glob' : 'Search', arg: String(i.pattern ?? i.query ?? '') };
  if (n === 'webfetch' || n === 'web_fetch') return { icon: 'globe', verb: 'Fetch', arg: String(i.url ?? '') };
  if (n === 'websearch' || n === 'web_search') return { icon: 'globe', verb: 'Web search', arg: String(i.query ?? '') };
  if (n === 'todowrite' || n === 'update_plan') return { icon: 'list-todo', verb: 'Plan', arg: 'checklist' };
  if (n === 'skill') return { icon: 'sparkles', verb: 'Skill', arg: String(i.skill ?? i.name ?? '') };
  if (/workflow/.test(n)) return { icon: 'gallery-vertical-end', verb: 'Workflow', arg: String(i.name ?? '') };
  if (isSpawn(tc)) return { icon: 'bot', verb: 'Spawn agent', arg: String(i.description ?? i.label ?? i.title ?? '') };
  return { icon: 'settings', verb: String(tc.name || 'Tool').replace(/^mcp__/, '').replace(/__/g, ' · '), arg: oneLine(firstString(i), 80) };
}

// ── edited-file accounting (lib/turnChanges.ts): estimates from tool input ─
function patchLines(diff) {
  let added = 0, removed = 0;
  const lines = String(diff || '').split('\n');
  for (let i = 0; i < lines.length; i++) {
    const l = lines[i];
    if (l.startsWith('***') || l.startsWith('@@')) continue;
    if (l.startsWith('--- ') && lines[i + 1] && lines[i + 1].startsWith('+++ ')) { i++; continue; }
    if (l.startsWith('+')) added++; else if (l.startsWith('-')) removed++;
  }
  return { added, removed };
}
export function editedFiles(calls) {
  const out = new Map();
  const bump = (p, a, r) => { if (!p) return; const e = out.get(p) || { added: 0, removed: 0 }; e.added += a; e.removed += r; out.set(p, e); };
  for (const tc of calls) {
    if (!EDIT.has(lower(tc.name))) continue;
    const i = tc.input || {};
    if (Array.isArray(i.changes)) { for (const ch of i.changes) { const c = patchLines(ch && ch.diff); bump(ch && ch.path, c.added, c.removed); } continue; }
    const p = i.file_path ?? i.path ?? i.filePath ?? i.notebook_path;
    if (!p) continue;
    if (typeof i.diff === 'string') { const c = patchLines(i.diff); bump(p, c.added, c.removed); continue; }
    let added = 0, removed = 0;
    const edits = Array.isArray(i.edits) ? i.edits : [{ old_string: i.old_string, new_string: i.new_string }];
    for (const e of edits) {
      if (typeof (e && e.old_string) === 'string' && e.old_string) removed += e.old_string.split('\n').length;
      if (typeof (e && e.new_string) === 'string' && e.new_string) added += e.new_string.split('\n').length;
    }
    if (typeof i.content === 'string' && i.content) added += i.content.split('\n').length;
    bump(p, added, removed);
  }
  return out;
}

function userHtml(t) {
  if (t.command) {
    const c = t.command;
    return `<div class="cmd"><div class="h">${ic('square-terminal', 's14 t-accent')}<b class="mono">/${esc(c.name)}${c.args ? ' ' + esc(c.args) : ''}</b></div>` +
      (c.output ? `<pre class="${c.outputIsError ? 't-error' : ''}">${esc(String(c.output).slice(0, 2000))}</pre>` : '') + '</div>';
  }
  const tags = [];
  const text = String(t.content || '').replace(/\[(Image|PDF|File):\s*([^\]]+)\]\s*/g, (_, kind, p) => {
    tags.push(`<span class="att">${ic(kind === 'Image' ? 'image' : 'file', 's12')}${esc(basename(p))}</span>`);
    return '';
  }).trim();
  return `<div class="you"><div class="lbl">You</div>${tags.length ? `<div class="atts">${tags.join('')}</div>` : ''}<div class="tx">${esc(text)}</div></div>`;
}

function assistantHtml(text, ctx) {
  return assistantBlocks(text).map((b) => {
    if (b.kind === 'card') {
      const card = parseCard(b.text);
      return card ? cardHtml(card) : `<div class="prose">${renderMarkdown('```wks-html-card\n' + b.text + '\n```')}</div>`;
    }
    if (b.kind === 'result') return resultCardHtml(b.text, '', { openKey: ctx.key('result'), open: ctx.open });
    return `<div class="prose">${renderMarkdown(b.text)}</div>`;
  }).join('');
}

function toolRow(tc, key, open) {
  const t = toolInfo(tc);
  const state = tc.status === 'running' ? spinner() : tc.status === 'failed' ? ic('circle-x', 's14 t-error') : ic('check', 's12 t-success');
  const k = `${key}:${tc.id || t.verb + t.arg}`;
  const isOpen = open.has(k);
  const detail = isOpen ? `<pre class="tout">${esc(tc.output ? String(tc.output) : JSON.stringify(tc.input ?? {}, null, 2).slice(0, 4000))}</pre>` : '';
  return `<button class="toolrow" data-toggle="${esc(k)}">${ic(t.icon, 's14 t-accent')}<span class="v">${esc(t.verb)}</span>` +
    `<span class="a mono">${esc(t.arg)}</span><span class="r">${state}</span></button>${detail}`;
}

/** A run of tool calls: one row, "Edited 3 files · 7 tools · 1m 12s ›". */
function workHtml(calls, key, open) {
  const edited = editedFiles(calls);
  const shells = calls.filter((c) => SHELL.has(lower(c.name))).length;
  const reads = calls.filter((c) => READ.has(lower(c.name))).length;
  const running = calls.some((c) => c.status === 'running');
  const failed = calls.some((c) => c.status === 'failed');
  const head = running ? 'Working' : edited.size ? `Edited ${edited.size} file${edited.size === 1 ? '' : 's'}`
    : shells ? `Ran ${shells} command${shells === 1 ? '' : 's'}` : reads ? `Read ${reads} file${reads === 1 ? '' : 's'}`
    : `Used ${calls.length} tool${calls.length === 1 ? '' : 's'}`;
  const start = Math.min(...calls.map((c) => c.startedAt || Infinity));
  const end = Math.max(...calls.map((c) => c.completedAt || c.startedAt || 0));
  const dur = Number.isFinite(start) && end > start ? ' · ' + fmtElapsed(end - start) : '';
  const isOpen = open.has(key);
  const icon = running ? spinner() : failed ? ic('circle-x', 's14 t-error') : ic('circle-check', 's14 t-success');
  return `<div class="work${isOpen ? ' open' : ''}"><button class="workrow" data-toggle="${esc(key)}" aria-expanded="${isOpen}">${icon}<span class="h">${esc(head)}</span>` +
    `<span class="r">${calls.length} tool${calls.length === 1 ? '' : 's'}${dur} ${ic(isOpen ? 'chevron-down' : 'chevron-right', 's14')}</span></button>` +
    (isOpen ? `<div class="tools">${calls.map((c) => toolRow(c, key, open)).join('')}</div>` : '') + '</div>';
}

/** Spawn agent: the native tool card with its live child row. */
function spawnHtml(tc, s) {
  const i = tc.input || {};
  const target = String(i.description ?? i.label ?? i.title ?? '');
  const prompt = String(i.prompt ?? i.message ?? i.task ?? '');
  const child = childrenOf(s).find((c) => c.toolUseId && c.toolUseId === tc.id) ||
    childrenOf(s).find((c) => !c.toolUseId && c.description && c.description === target);
  // A workspacer spawn_agent tool starts a whole session under this one.
  const spawned = !child && /spawn_agent$/.test(String(tc.name || ''))
    ? [...sessions.values()].find((x) => x.parentSessionId === s.sessionId && (x.label === (i.label || i.title) || x.label === target))
    : null;
  const badge = tc.status === 'failed' ? '<span class="badge error">Failed</span>'
    : tc.status === 'running' && !child && !spawned ? `<span class="badge busy">${ic('loader-circle', 's12 spinning')}Starting</span>`
    : `<span class="badge success">${ic('check', 's12')}Dispatched</span>`;
  let row = '';
  if (child) {
    const st = childStatus(child);
    row = `<button class="childrow" data-child-open="${esc(s.sessionId)}" data-agent="${esc(child.id)}">
      <span class="a">${mark(providerOf(s))}${childActive(child) && st.label === 'Working' ? spinner() : ''}<span>${esc(child.description || target || 'Subagent')}</span><span class="st t-${st.tone}">${esc(st.label)} ${ic('arrow-right', 's12')}</span></span>
      <span class="b">${esc([child.model && modelName(child.model), child.tokens && `${Math.round(child.tokens / 100) / 10}k tokens`, child.lastToolName && `${child.lastToolName}${child.lastToolSummary ? ' · ' + child.lastToolSummary : ''}`].filter(Boolean).join(' · '))}</span></button>`;
  } else if (spawned) {
    const st = status(spawned);
    row = `<button class="childrow" data-open="${esc(spawned.sessionId)}"><span class="a">${mark(providerOf(spawned))}<span>${esc(titleOf(spawned))}</span><span class="st t-${st.tone}">${esc(st.label)} ${ic('arrow-right', 's12')}</span></span></button>`;
  }
  return `<div class="toolcard"><div class="h">${ic('bot', 's14 t-accent')}<b>Spawn agent</b><span class="tg">${esc(target)}</span>${badge}</div>` +
    (prompt ? `<div class="quote">${esc(prompt.length > 280 ? prompt.slice(0, 280).trimEnd() + '…' : prompt)}</div>` : '') + row + '</div>';
}

function filesHtml(edited, cwd, key) {
  let added = 0, removed = 0;
  for (const e of edited.values()) { added += e.added; removed += e.removed; }
  const strip = (p) => { const c = String(cwd || '').replace(/\/+$/, ''); return c && p.startsWith(c + '/') ? p.slice(c.length + 1) : basename(p); };
  const names = [...edited.keys()].slice(0, 3).map(strip).join(' · ') + (edited.size > 3 ? ` · +${edited.size - 3}` : '');
  return `<button class="files" data-changes="${esc(key)}"><span class="a">${edited.size} file${edited.size === 1 ? '' : 's'} changed` +
    `<span class="r"><span class="t-success">+${added}</span><span class="t-error">−${removed}</span><span class="muted cl">${ic('arrow-right', 's12')}Changes</span></span></span>` +
    `<span class="names muted">${esc(names)}</span></button>`;
}

function traceHtml(r) {
  if (r.declined) return `<div class="trace no">${ic('circle-x', 's14')}<span>You declined to answer — the turn was cancelled.</span></div>`;
  return `<div class="trace">${ic('circle-check', 's14 t-success')}<div>${r.questions.map((q, i) =>
    `<div class="q">${esc(q.question)}</div><div class="a">↳ ${esc((r.answers && r.answers[i]) || '—')}</div>`).join('')}</div></div>`;
}

/** Render turns. `ctx`: { s, open:Set, resolved:[], pending:[], editedByKey:Map, readOnly } */
export function transcriptHtml(turns, ctx) {
  const { s, open } = ctx;
  if (!turns.length) return '';
  const out = [];
  let work = [], groupCalls = [], group = 0, seq = 0, groupStart = 0, groupEnd = 0, sawAssistant = false;
  const anchors = new Map();
  for (const r of ctx.resolved || []) {
    const a = Math.max(0, Math.min(r.anchorLen, turns.length));
    if (!anchors.has(a)) anchors.set(a, []);
    anchors.get(a).push(r);
  }
  const flushWork = () => {
    if (!work.length) return;
    const key = `${s.sessionId}:w:${work[0].id || group + '.' + seq}`;
    seq++;
    out.push(workHtml(work, key, open));
    work = [];
  };
  const flushGroup = (last) => {
    flushWork();
    const edited = editedFiles(groupCalls);
    if (edited.size) {
      const key = `${s.sessionId}:g:${group}`;
      ctx.editedByKey.set(key, edited);
      out.push(filesHtml(edited, s.cwd, key));
    }
    // "Took" only for a finished turn: not the live one.
    if (sawAssistant && groupStart && groupEnd > groupStart && !(last && ctx.working)) {
      out.push(`<div class="meta-line"><span>Took ${esc(fmtElapsed(groupEnd - groupStart))}</span><span>${esc(when(groupEnd))}</span></div>`);
    }
    groupCalls = []; group++; seq = 0; sawAssistant = false;
  };
  const keyFor = (i) => (kind) => `${s.sessionId}:${i}:${kind}`;
  for (let i = 0; i < turns.length; i++) {
    for (const r of anchors.get(i) || []) { flushWork(); out.push(traceHtml(r)); }
    const t = turns[i];
    if (t.role === 'user') {
      flushGroup(false);
      groupStart = t.timestamp || 0; groupEnd = groupStart;
      const fleet = !t.command && parseFleetMessage(t.content);
      out.push(fleet ? fleetCardHtml(fleet, { openKey: `${s.sessionId}:${i}`, open, known: (id) => sessions.has(id) }) : userHtml(t));
      continue;
    }
    sawAssistant = true;
    if (t.timestamp) groupEnd = Math.max(groupEnd, t.timestamp);
    const text = String(t.content || '').trim();
    if (text) { flushWork(); out.push(assistantHtml(t.content, { key: keyFor(i), open })); }
    for (const tc of t.toolCalls || []) {
      groupCalls.push(tc);
      if (tc.completedAt) groupEnd = Math.max(groupEnd, tc.completedAt);
      if (isSpawn(tc)) { flushWork(); out.push(spawnHtml(tc, s)); continue; }
      work.push(tc);
    }
  }
  flushGroup(true);
  for (const r of anchors.get(turns.length) || []) out.push(traceHtml(r));
  return out.join('');
}
