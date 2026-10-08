// Chat: native's conversation in a phone. A title island at the top (tap it
// for the session menu), the transcript, and a dock holding whatever needs
// you (question / approval), the paused + cache-cold note, and the composer.
import { bus, can, call } from '../bus.js';
import {
  sessions, status, isLive, isWorking, isPaused, canResume, hubDown, hubOf, offlineHubs, titleOf, modelOf,
  providerOf, permissionOf, context, gaugeTone, coldCache, LARGE_CONTEXT, turnsOf, convOf, watchConversation,
  resolved, questionSig, runningTasks, isArchived, drafts,
} from '../store.js';
import { esc, ic, mark, spinner, fmtElapsed, fmtTokens, fmtUSD, ago, modelName, errText, copyText } from '../util.js';
import { transcriptHtml } from './transcript.js';
import * as act from '../actions.js';
import { notice, stickToBottom, sheet, pick } from '../ui.js';
import { openMenu, openModelPicker, openHandoff, openChanges, openDiff } from './island.js';
import { openTasks } from './tasks.js';

const pendingSends = new Map();   // sessionId -> [{text, at, queued}]
const dismissedCold = new Set();  // sessionId@expiresAt
const qstate = new Map();         // sessionId -> {sig, page, picks: Map<qi, Set<oi>>, text: Map<qi, string>}
const workingSince = new Map();   // sessionId -> first seen working (for "Working · 18s")

function summarizeInput(input) {
  if (input == null) return '';
  if (typeof input === 'string') return input;
  if (input.command) return Array.isArray(input.command) ? input.command.join(' ') : String(input.command);
  if (input.file_path) return input.file_path + (typeof input.content === 'string' ? '\n' + input.content.slice(0, 400) : '');
  if (input.path) return String(input.path);
  if (input.url) return String(input.url);
  if (input.pattern) return String(input.pattern);
  try { return JSON.stringify(input, null, 2).slice(0, 600); } catch { return String(input); }
}

export function mount(root, ctx, sessionId) {
  root.innerHTML = `<div class="screen chat">
    <div class="chattop">
      <button class="ibtn round" data-back aria-label="Back">${ic('chevron-left', 's20')}</button>
      <div class="island" data-island role="button" tabindex="0" aria-label="Session menu"></div>
      <span data-right></span>
    </div>
    <div class="transcript scrolly" data-scroll><div class="thread" data-tx></div></div>
    <div class="dock" data-dock>
      <div data-cards></div>
      <div class="composer" data-composer>
        <div class="atts" data-atts></div>
        <textarea rows="1" data-input aria-label="Message" enterkeyhint="enter"></textarea>
        <div class="cr"><button class="ibtn" data-plus aria-label="Attach">${ic('plus', 's18')}</button><span class="grow"></span><span data-send></span></div>
      </div>
      <div class="dockline" data-line></div>
    </div>
    <input type="file" accept="image/*" multiple hidden data-file>
  </div>`;
  const $ = (s) => root.querySelector(s);
  const scroll = $('[data-scroll]'), tx = $('[data-tx]'), dock = $('[data-dock]'), input = $('[data-input]');
  const pin = stickToBottom(scroll);
  const open = new Set();
  const editedByKey = new Map();
  let attach = [], attachSeq = 0;
  let sending = false;

  input.value = drafts.get(sessionId) || '';
  const autosize = () => { input.style.height = 'auto'; input.style.height = Math.min(input.scrollHeight, 160) + 'px'; };
  input.addEventListener('input', () => { drafts.set(sessionId, input.value); autosize(); renderDock(); });
  autosize();

  // The transcript scrolls under the dock; keep its bottom clear of it.
  const ro = new ResizeObserver(() => { scroll.style.setProperty('--dock-h', dock.offsetHeight + 'px'); pin.after(); });
  ro.observe(dock);

  $('[data-back]').onclick = () => ctx.back();
  const island = $('[data-island]');
  island.onclick = (e) => {
    if (e.target.closest('[data-model]')) return;
    const s = sessions.get(sessionId);
    if (s) openMenu(s, { go: ctx.go, onDraft: setDraft });
  };
  island.onkeydown = (e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); island.click(); } };
  $('[data-file]').onchange = (e) => { const files = [...(e.target.files || [])]; e.target.value = ''; attachFiles(files); };

  function setDraft(text) { drafts.set(sessionId, text); input.value = text; autosize(); input.focus(); renderDock(); }

  // ── delegated clicks (transcript + dock) ───────────────────────────────
  root.addEventListener('click', async (e) => {
    const t = e.target.closest('button, [data-toggle]');
    if (!t || !root.contains(t)) return;
    const s = sessions.get(sessionId);
    if (t.dataset.toggle) { open.has(t.dataset.toggle) ? open.delete(t.dataset.toggle) : open.add(t.dataset.toggle); renderTranscript(); return; }
    if (t.dataset.open) { ctx.go('#/s/' + encodeURIComponent(t.dataset.open)); return; }
    if (t.dataset.childOpen) { ctx.go(`#/s/${encodeURIComponent(t.dataset.childOpen)}/a/${encodeURIComponent(t.dataset.agent)}`); return; }
    if (t.dataset.mention) { setDraft((input.value ? input.value + ' ' : '') + `session:${t.dataset.mention} `); return; }
    if (t.dataset.copy !== undefined && t.dataset.copy) { notice((await copyText(t.dataset.copy)) ? 'Copied' : 'Copy failed'); return; }
    if (t.hasAttribute('data-copy-code')) { const code = t.closest('.codeblock').querySelector('code').textContent; notice((await copyText(code)) ? 'Copied' : 'Copy failed'); return; }
    if (t.dataset.cardAction !== undefined && t.dataset.kind) {
      const v = t.dataset.value;
      if (t.dataset.kind === 'fill_composer') setDraft(v);   // never sends
      else if (t.dataset.kind === 'open_worker') { if (sessions.has(v)) ctx.go('#/s/' + encodeURIComponent(v)); else notice('That worker is not on this hub', 'warning'); }
      else if (t.dataset.kind === 'view_diff' && s) openDiff(s, v);
      return;
    }
    if (t.dataset.changes && s) { openChanges(s, editedByKey.get(t.dataset.changes)); return; }
    if (t.dataset.model !== undefined && s) { openModelPicker(s); return; }
    if (t.hasAttribute('data-tasks') && s) { openTasks(s, { go: ctx.go }); return; }
    if (t.hasAttribute('data-menu') && s) { openMenu(s, { go: ctx.go, onDraft: setDraft }); return; }
    if (t.hasAttribute('data-plus')) { plusSheet(); return; }
    if (t.hasAttribute('data-send')) { submit(); return; }
    if (t.hasAttribute('data-stop') && s) { act.interrupt(sessionId); return; }
    if (t.dataset.rmAtt) { attach = attach.filter((a) => a.id !== t.dataset.rmAtt); renderAtts(); renderDock(); return; }
    if (t.dataset.approve && s) { t.disabled = true; await act.approve(sessionId, t.dataset.approve); return; }
    if (t.hasAttribute('data-details')) { open.has('approval') ? open.delete('approval') : open.add('approval'); renderDock(); return; }
    if (t.dataset.opt !== undefined && s) { pickOption(s, Number(t.dataset.q), Number(t.dataset.opt)); return; }
    if (t.hasAttribute('data-qnext')) { const q = qs(sessions.get(sessionId)); q.page++; renderDock(); return; }
    if (t.hasAttribute('data-qprev')) { const q = qs(sessions.get(sessionId)); q.page = Math.max(0, q.page - 1); renderDock(); return; }
    if (t.hasAttribute('data-qsend') && s) { sendAnswers(s); return; }
    if (t.hasAttribute('data-decline') && s) { act.declineQuestion(sessionId); return; }
    if (t.hasAttribute('data-fresh') && s) { openHandoff(s, { fresh: true, go: ctx.go }); return; }
    if (t.hasAttribute('data-cold-x') && s) { dismissedCold.add(sessionId + '@' + s.promptCache.expiresAt); renderDock(); return; }
    if (t.hasAttribute('data-open-parent')) { ctx.back(); return; }
  });
  root.addEventListener('input', (e) => {
    const el = e.target.closest('[data-qtext]');
    if (!el) return;
    const q = qs(sessions.get(sessionId));
    const qi = Number(el.dataset.qtext);
    q.text.set(qi, el.value);
    if (el.value.trim()) q.picks.set(qi, new Set());
    renderDockFooter();
  });
  input.addEventListener('keydown', (e) => {
    // Enter is a newline on a phone; Ctrl/Cmd+Enter sends from a keyboard.
    if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) { e.preventDefault(); submit(); }
  });
  input.addEventListener('paste', (e) => {
    const imgs = [...((e.clipboardData && e.clipboardData.items) || [])].filter((i) => i.kind === 'file' && /^image\//.test(i.type)).map((i) => i.getAsFile()).filter(Boolean);
    if (imgs.length) { e.preventDefault(); attachFiles(imgs); }
  });

  // ── questions ──────────────────────────────────────────────────────────
  function qs(s) {
    const sig = questionSig(s);
    let q = qstate.get(sessionId);
    if (!q || q.sig !== sig) { q = { sig, page: 0, picks: new Map(), text: new Map() }; qstate.set(sessionId, q); }
    return q;
  }
  const answeredQ = (q, qi) => (q.picks.get(qi) && q.picks.get(qi).size > 0) || !!(q.text.get(qi) || '').trim();
  function pickOption(s, qi, oi) {
    const q = qs(s);
    const question = s.pendingQuestions[qi];
    const set = new Set(question.multiSelect ? q.picks.get(qi) || [] : []);
    if (question.multiSelect && set.has(oi)) set.delete(oi); else set.add(oi);
    q.picks.set(qi, set);
    q.text.delete(qi);
    renderDock();
  }
  async function sendAnswers(s) {
    const q = qs(s);
    const questions = s.pendingQuestions || [];
    if (!questions.every((_, qi) => answeredQ(q, qi))) { notice('Answer every question first', 'warning'); return; }
    // Native sends each answer as its label text (answerKinds "text").
    const labels = questions.map((question, qi) => {
      const typed = (q.text.get(qi) || '').trim();
      if (typed) return typed;
      return [...q.picks.get(qi)].sort((a, b) => a - b).map((oi) => (question.options[oi] || {}).label).filter(Boolean).join(', ');
    });
    const ok = await act.answer(sessionId, labels, labels);
    if (ok) qstate.delete(sessionId);
  }

  // ── attachments ────────────────────────────────────────────────────────
  async function attachFiles(files) {
    if (!can('files.upload')) { notice('Attaching needs a token that can upload files', 'warning'); return; }
    for (const file of files) {
      if (!file || !/^image\//.test(file.type || '')) continue;
      const entry = { id: 'a' + (++attachSeq), name: file.name || 'photo', path: '', thumb: '', uploading: true };
      attach.push(entry); renderAtts(); renderDock();
      try {
        const { name, dataUrl, thumb } = await act.normalizeImage(file);
        entry.thumb = thumb; renderAtts();
        entry.path = await act.upload(sessionId, name, dataUrl);
        entry.name = name; entry.uploading = false;
      } catch (e) {
        attach = attach.filter((a) => a.id !== entry.id);
        notice('Attach failed: ' + errText(e), 'error');
      }
      renderAtts(); renderDock();
    }
  }
  function renderAtts() {
    $('[data-atts]').innerHTML = attach.map((a) => `<span class="att${a.uploading ? ' up' : ''}">${a.thumb ? `<img src="${a.thumb}" alt="">` : ic('image', 's14')}` +
      `<span>${esc(a.uploading ? 'Uploading…' : a.name)}</span><button data-rm-att="${a.id}" aria-label="Remove">${ic('x', 's12')}</button></span>`).join('');
  }
  function plusSheet() {
    const s = sessions.get(sessionId);
    sheet(`<div class="sh"><h3>Add to message</h3><button class="ibtn" data-close aria-label="Close">${ic('x', 's18')}</button></div>
      <div class="group menu">
        <button class="mi" data-photo${can('files.upload') ? '' : ' disabled'}>${ic('image')}Photo or screenshot</button>
        <button class="mi" data-lib${can('library.list') ? '' : ' disabled'}>${ic('book-open')}Saved prompt</button>
      </div>`, {
      label: 'Add to message',
      bind(el, close) {
        el.querySelector('[data-photo]').onclick = () => { close(); $('[data-file]').click(); };
        el.querySelector('[data-lib]').onclick = async () => {
          close();
          try {
            const prompts = ((await call('library.list', { cwd: s && s.cwd })) || []).filter((i) => !i.kind || i.kind === 'prompt').slice(0, 30);
            const chosen = await pick('Saved prompts', prompts.map((p) => ({ value: p.body || p.title || '', label: p.title || p.id })));
            if (chosen) setDraft(chosen);
          } catch (err) { notice(errText(err), 'error'); }
        };
      },
    });
  }

  // ── send / resume ──────────────────────────────────────────────────────
  async function submit() {
    const s = sessions.get(sessionId);
    if (!s || sending) return;
    if (attach.some((a) => a.uploading)) { notice('Still uploading…'); return; }
    const typed = input.value.trim();
    const prefix = attach.filter((a) => a.path).map((a) => `[Image: ${a.path}]`).join(' ');
    const text = prefix ? prefix + (typed ? ' ' + typed : '') : typed;
    if (!text) return;
    sending = true; renderDock();
    let ok;
    if (!isLive(s) && canResume(s)) {
      ok = await act.resumeWith(sessionId, text);
      if (ok && ok !== sessionId) { clearComposer(); sending = false; ctx.go('#/s/' + encodeURIComponent(ok), true); return; }
    } else {
      ok = await act.send(sessionId, text);
    }
    sending = false;
    if (ok) {
      const list = pendingSends.get(sessionId) || [];
      list.push({ text, at: Date.now(), queued: isWorking(s) });
      pendingSends.set(sessionId, list);
      clearComposer();
      pin.pin();
    }
    render();
  }
  function clearComposer() { input.value = ''; drafts.delete(sessionId); attach = []; renderAtts(); autosize(); }

  // ── render ─────────────────────────────────────────────────────────────
  function renderIsland(s) {
    const st = status(s);
    const c = context(s);
    const model = modelName(modelOf(s)) || (providerOf(s) === 'codex' ? 'Codex' : 'Claude');
    const dot = isWorking(s) ? 'busy' : st.tone;
    island.innerHTML = `<span class="dot t-${dot}"></span><span class="tt">${esc(titleOf(s))}</span>` +
      `<button class="mchip ${providerOf(s) === 'claude' ? '' : 'other'}" data-model aria-label="Model ${esc(model)}">${mark(providerOf(s), 's12')}${esc(model.split(' ').slice(0, 2).join(' '))}${act.canSwitch() && isLive(s) ? ic('chevron-down', 's12') : ''}</button>` +
      (c ? `<span class="gauge" aria-label="Context ${Math.round(c.pct)}%"><i style="width:${Math.min(100, c.pct)}%;background:var(--wks-${gaugeTone(c.pct)})"></i></span>` : '');
    const n = runningTasks(s);
    $('[data-right]').innerHTML = n
      ? `<button class="tchip" data-tasks aria-label="${n} background task${n === 1 ? '' : 's'}">${ic('square-terminal', 's14')}${n}</button>`
      : `<button class="ibtn round" data-menu aria-label="More">${ic('ellipsis', 's18')}</button>`;
  }

  function renderTranscript() {
    const s = sessions.get(sessionId);
    if (!s) return;
    const turns = turnsOf(s);
    const st = convOf(sessionId);
    editedByKey.clear();
    let html = transcriptHtml(turns, { s, open, resolved: resolved.get(sessionId), editedByKey, working: isWorking(s) });
    // Sent messages wait here until the transcript acknowledges them.
    const pend = (pendingSends.get(sessionId) || []).filter((p) => {
      const seen = turns.slice(-8).some((t) => t.role === 'user' && String(t.content || '').trim() === p.text);
      return !seen && Date.now() - p.at < 120000;
    });
    pendingSends.set(sessionId, pend);
    html += pend.map((p) => `<div class="you pending"><div class="lbl">You</div><div class="tx">${esc(p.text)}</div>` +
      `<div class="meta">${p.queued ? 'Queued — sends when this turn ends' : 'Sent'}</div></div>`).join('');
    if (!html) {
      html = !st.loaded && !(s.conversation && s.conversation.length)
        ? `<div class="empty">${spinner()}<p>Loading the conversation…</p></div>`
        : `<div class="empty">${ic('message-square-plus', 's20 muted')}<b>No messages yet</b><p>${isLive(s) ? 'Say what you want done.' : 'This session has no conversation to show.'}</p></div>`;
    }
    if (st.error && !turns.length) html += `<div class="empty"><p class="t-error">${esc(st.error)}</p></div>`;
    const top = scroll.scrollTop;
    tx.innerHTML = html;
    if (pin.pinned) pin.after(); else scroll.scrollTop = top;
  }

  function cardsHtml(s) {
    if (hubDown(s)) {
      return `<div class="note">${ic('wifi-off', 's16 muted')}<div><div class="nt">${esc(hubOf(s))} is offline</div><div class="nb">Last seen ${esc(ago(Date.now() - (offlineHubs.get(hubOf(s)) || Date.now())))}. This session is read-only until its hub reconnects.</div></div></div>`;
    }
    if (s.pendingQuestions && s.pendingQuestions.length) return questionHtml(s);
    if (s.pendingApproval) return approvalHtml(s);
    const notes = [];
    if (!isLive(s) && canResume(s)) {
      const access = act.accessLabel(act.accessWire(providerOf(s) === 'codex' ? 'codex' : 'claude', permissionOf(s)));
      const model = modelName(modelOf(s));
      notes.push(isPaused(s)
        ? `<div class="ns">${ic('pause', 's16')}<div><div class="nt">Paused when the app closed</div><div class="nb">Send a message to pick up where it left off: same conversation${model ? ', ' + esc(model) : ''}, ${esc(access)}.</div></div></div>`
        : `<div class="ns">${ic('circle-stop', 's16')}<div><div class="nt">This session ended</div><div class="nb">Send a message to resume it: same conversation${model ? ', ' + esc(model) : ''}, ${esc(access)}.</div></div></div>`);
    }
    const cold = coldCache(s);
    if (cold && (cold.contextTokens || 0) >= LARGE_CONTEXT && !dismissedCold.has(sessionId + '@' + cold.expiresAt)) {
      const when = ago(Date.now() - cold.expiresAt);
      const head = cold.estimated ? `Cache likely expired ${when} (estimate)` : `Cache expired ${when}`;
      const cost = cold.coldCostUSD != null ? ` (≈${fmtUSD(cold.coldCostUSD)}${cold.warmCostUSD != null ? `, vs ${fmtUSD(cold.warmCostUSD)} warm` : ''})` : '';
      const fresh = ['claude', 'codex'].includes(providerOf(s)) && !s.isWakeTarget && can('agents.spawn');
      notes.push(`<div class="ns cold" data-cold>${ic('snowflake', 's16')}<div class="grow"><div class="nt">${esc(head)}</div>` +
        `<div class="nb">Your next message re-sends ~${esc(fmtTokens(cold.contextTokens))} tokens${esc(cost)}.</div>` +
        (fresh ? `<div class="na"><button class="btn sm" data-fresh>${ic('sparkles', 's14')}Start fresh from a summary</button></div>` : '') +
        `</div><button class="ibtn sm" data-cold-x aria-label="Dismiss">${ic('x', 's14')}</button></div>`);
    }
    if (notes.length) return `<div class="note stack${isPaused(s) ? ' paused' : ''}">${notes.join('<div class="hair"></div>')}</div>`;
    return '';
  }

  function approvalHtml(s) {
    const a = s.pendingApproval;
    const input = a.toolInput || {};
    const desc = typeof input.description === 'string' ? input.description : '';
    const body = summarizeInput(input);
    const details = open.has('approval');
    const allowed = can('claude.approve');
    return `<div class="ask" role="region" aria-label="Needs approval">
      <div class="ah"><span class="dot"></span>Needs approval</div>
      <div class="ap">${ic(/bash|shell|command/i.test(a.toolName || '') ? 'square-terminal' : 'shield-check', 's18 t-accent')}<div><div class="tn">${esc(a.toolName || 'Tool')}</div>${desc ? `<div class="td muted">${esc(desc)}</div>` : ''}</div></div>
      ${body ? `<div class="codeblock"><pre>${esc(body)}</pre></div>` : ''}
      <button class="disc" data-details>${ic(details ? 'chevron-down' : 'chevron-right', 's14')}Details</button>
      ${details ? `<pre class="tout">${esc(JSON.stringify(input, null, 2))}</pre>` : ''}
      <div class="row2 ratio"><button class="btn block" data-approve="no"${allowed ? '' : ' disabled'}>Deny</button><button class="btn block primary" data-approve="yes"${allowed ? '' : ' disabled'}>Allow once</button></div>
      ${allowed ? '' : '<div class="faint small">This device’s token cannot approve tools.</div>'}
    </div>`;
  }

  function questionHtml(s) {
    const questions = s.pendingQuestions;
    const q = qs(s);
    q.page = Math.min(q.page, questions.length - 1);
    const qi = q.page;
    const question = questions[qi];
    const picks = q.picks.get(qi) || new Set();
    const n = questions.length;
    const done = questions.filter((_, i) => answeredQ(q, i)).length;
    const opts = (question.options || []).map((o, oi) => `<button class="opt${picks.has(oi) ? ' on' : ''}" data-q="${qi}" data-opt="${oi}" aria-pressed="${picks.has(oi)}">
        <span class="num">${oi + 1}</span><span><span class="ot">${esc(o.label)}</span>${o.description ? `<span class="od">${esc(o.description)}</span>` : ''}</span></button>`).join('');
    const can_ = can('claude.answer');
    return `<div class="ask" role="region" aria-label="Needs your input">
      <div class="ah"><span class="dot"></span>Needs your input${n > 1 ? ` · ${n} questions` : ''}${n > 1 ? `<span class="r">${done} of ${n} answered</span>` : ''}</div>
      <div class="qh">${n > 1 ? `<span class="overline">${qi + 1} of ${n}${question.header ? ' · ' + esc(question.header) : ''}</span>` : question.header ? `<span class="overline">${esc(question.header)}</span>` : ''}
        <span class="muted small">${question.multiSelect ? 'Choose any' : 'Choose one'}</span></div>
      <div class="q">${esc(question.question)}</div>
      <div class="opts">${opts}</div>
      <input class="qother" data-qtext="${qi}" placeholder="Or type a different answer" value="${esc(q.text.get(qi) || '')}" aria-label="Or type a different answer">
      <div class="af" data-qfooter>${footerHtml(s, q, can_)}</div>
    </div>`;
  }
  function footerHtml(s, q, allowed) {
    const n = s.pendingQuestions.length, qi = q.page;
    const left = s.pendingQuestions.filter((_, i) => !answeredQ(q, i)).length;
    const pager = n > 1 ? `<span class="pager">${s.pendingQuestions.map((_, i) => `<i class="${i === qi ? 'on' : answeredQ(q, i) ? 'done' : ''}"></i>`).join('')}</span>` : '';
    const hint = left ? `Answer ${left} more question${left === 1 ? '' : 's'} to send` : 'Ready to send';
    const last = qi === n - 1 || (left === 0);
    const btn = !allowed ? '<button class="btn sm" disabled>Read only</button>'
      : !last ? `<button class="btn sm primary" data-qnext${answeredQ(q, qi) ? '' : ' disabled'}>Next ${ic('arrow-right', 's14')}</button>`
      : `<button class="btn sm primary" data-qsend${left ? ' disabled' : ''}>${n > 1 ? 'Send answers' : 'Send answer'}</button>`;
    return `${qi > 0 ? `<button class="ibtn sm" data-qprev aria-label="Previous question">${ic('chevron-left', 's16')}</button>` : ''}${pager}<span class="grow">${esc(hint)}</span>` +
      `${allowed && can('claude.signal') ? '<button class="btn sm quiet" data-decline>Decline</button>' : ''}${btn}`;
  }
  function renderDockFooter() {
    const s = sessions.get(sessionId);
    const f = root.querySelector('[data-qfooter]');
    if (s && f && s.pendingQuestions) f.innerHTML = footerHtml(s, qs(s), can('claude.answer'));
  }

  let lastCards = '';
  function renderDock() {
    const s = sessions.get(sessionId);
    if (!s) return;
    const cards = cardsHtml(s);
    // Rebuilding a card under a focused field would drop the typing.
    const typing = document.activeElement && document.activeElement.matches('[data-qtext]');
    if (cards !== lastCards && !typing) { $('[data-cards]').innerHTML = cards; lastCards = cards; }
    const live = isLive(s), working = isWorking(s), resumable = !live && canResume(s);
    const readOnly = hubDown(s) || (!live && !resumable) || !can(resumable ? 'agents.spawn' : 'agents.sendMessage');
    input.disabled = readOnly;
    input.placeholder = hubDown(s) ? 'Hub offline' : !live && !resumable ? 'This session has ended' : resumable ? 'Send a message to resume…' : working ? 'Add to the queue…' : 'Ask anything, or describe a task…';
    const hasText = !!input.value.trim() || attach.some((a) => a.path);
    $('[data-send]').innerHTML = working && !hasText
      ? `<button class="send stop" data-stop aria-label="Stop">${ic('square', 's14')}</button>`
      : `<button class="send${hasText && !sending && !readOnly ? '' : ' off'}" data-send aria-label="${resumable ? 'Resume and send' : 'Send'}"${hasText && !sending && !readOnly ? '' : ' disabled'}>${sending ? spinner() : ic('arrow-up', 's18')}</button>`;
    // status line
    let line = '';
    if (!bus.connected) line = `<span class="t-warning">Reconnecting…</span>`;
    else if (working) {
      // The turn started at the last message you sent, when the transcript
      // has it; otherwise when this phone first saw it working.
      if (!workingSince.has(sessionId)) workingSince.set(sessionId, Date.now());
      const lastUser = [...turnsOf(s)].reverse().find((t) => t.role === 'user' && t.timestamp);
      const since = lastUser && Date.now() - lastUser.timestamp < 6 * 3600000 ? lastUser.timestamp : workingSince.get(sessionId);
      line = `${spinner()}<span>Working · ${esc(fmtElapsed(Date.now() - since))}</span>${hasText ? '<span class="r">Queues until this turn ends</span>' : ''}`;
    } else if (s.pendingApproval) line = '<span>Waiting for approval…</span>';
    else if (s.pendingQuestions && s.pendingQuestions.length) line = '<span>Waiting for your answer…</span>';
    else if (resumable) line = isPaused(s) ? '<span class="t-accent">Paused</span><span>· sending resumes this session</span>' : '<span>Ended · sending resumes this session</span>';
    else if (!live) line = '<span>Ended</span>';
    if (!working) workingSince.delete(sessionId);
    $('[data-line]').innerHTML = line;
  }

  function render() {
    const s = sessions.get(sessionId);
    if (!s) {
      tx.innerHTML = `<div class="empty">${spinner()}<p>Looking for this session…</p></div>`;
      return;
    }
    if (isArchived(sessionId)) island.classList.add('archived'); else island.classList.remove('archived');
    renderIsland(s);
    renderTranscript();
    renderDock();
  }

  watchConversation(sessionId);
  renderAtts();
  render();
  pin.pin();
  const tick = setInterval(() => { if (document.visibilityState === 'visible') renderDock(); }, 1000);
  return {
    update(ids) {
      if (ids.has('*') || ids.has(sessionId)) render();
      else renderDock();
    },
    destroy() { clearInterval(tick); ro.disconnect(); watchConversation(''); },
    sessionId,
  };
}

