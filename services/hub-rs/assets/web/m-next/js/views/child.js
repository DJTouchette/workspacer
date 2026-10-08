// A provider-native subagent (Claude's Task/Agent child), parent-scoped and
// read-only (native ui/children.rs): its own transcript through
// sessions.subagentConversation, the task its parent gave it, and a dock that
// says subagents take instructions from the parent.
import { sessions, childrenOf, childStatus, childActive, childConvOf, fetchChild, titleOf, providerOf } from '../store.js';
import { esc, ic, mark, spinner, fmtElapsed, modelName } from '../util.js';
import { transcriptHtml } from './transcript.js';
import { stickToBottom } from '../ui.js';

export function mount(root, ctx, parentId, agentId) {
  root.innerHTML = `<div class="screen chat child">
    <div class="chattop">
      <button class="ibtn round" data-back aria-label="Back">${ic('chevron-left', 's20')}</button>
      <div class="island" data-island></div>
      <span class="ibtn round ghost"></span>
    </div>
    <div class="transcript scrolly" data-scroll><div class="thread" data-tx></div></div>
    <div class="dock" data-dock>
      <div class="note row">${ic('info', 's16 muted')}<div class="nb grow">Subagents take their instructions from the parent session.</div><button class="btn sm primary" data-parent>Open parent</button></div>
      <div class="dockline" data-line></div>
    </div>
  </div>`;
  const $ = (s) => root.querySelector(s);
  const scroll = $('[data-scroll]');
  const pin = stickToBottom(scroll);
  const open = new Set();
  const ro = new ResizeObserver(() => { scroll.style.setProperty('--dock-h', $('[data-dock]').offsetHeight + 'px'); pin.after(); });
  ro.observe($('[data-dock]'));
  $('[data-back]').onclick = () => ctx.back();
  $('[data-parent]').onclick = () => ctx.go('#/s/' + encodeURIComponent(parentId), true);
  root.addEventListener('click', (e) => {
    const t = e.target.closest('[data-toggle]');
    if (t) { open.has(t.dataset.toggle) ? open.delete(t.dataset.toggle) : open.add(t.dataset.toggle); render(); return; }
    const p = e.target.closest('[data-goparent]');
    if (p) ctx.go('#/s/' + encodeURIComponent(parentId), true);
  });

  function render() {
    const parent = sessions.get(parentId);
    const child = parent && childrenOf(parent).find((c) => c.id === agentId);
    const st = child ? childStatus(child) : { label: 'Unavailable', tone: 'muted' };
    const name = (child && (child.description || child.type)) || 'Subagent';
    $('[data-island]').innerHTML = `<span class="dot t-${st.tone}"></span><span class="tt"><span class="pj">↳ </span>${esc(name)}</span>` +
      (child && child.model ? `<span class="mchip">${mark(parent ? providerOf(parent) : 'claude', 's12')}${esc(modelName(child.model).split(' ')[0])}</span>` : '');
    const conv = childConvOf(parentId, agentId);
    const crumb = `<button class="crumb" data-goparent><span class="peer">${ic('bot', 's12')}Subagent</span>of <b>${esc(parent ? titleOf(parent) : 'its parent')}</b>${ic('chevron-right', 's12')}</button>`;
    const task = child && (child.prompt || child.description)
      ? `<div class="toolcard"><div class="overline">Task from parent</div><div class="quote big">${esc(child.prompt || child.description)}</div></div>` : '';
    let body = transcriptHtml(conv.turns, { s: { sessionId: parentId + '/' + agentId, cwd: parent && parent.cwd }, open, editedByKey: new Map(), working: child && childActive(child) });
    if (!body) body = conv.loaded ? `<div class="empty"><p>${esc(conv.error || 'Nothing to show yet.')}</p></div>` : `<div class="empty">${spinner()}<p>Loading…</p></div>`;
    $('[data-tx]').innerHTML = crumb + task + body;
    pin.after();
    const elapsed = child && child.startedAt ? fmtElapsed((child.completedAt || Date.now()) - child.startedAt) : '';
    $('[data-line]').innerHTML = child && childActive(child) && st.label === 'Working'
      ? `${spinner()}<span>Working${elapsed ? ' · ' + esc(elapsed) : ''}</span><span class="r">Clear when finished</span>`
      : `<span class="t-${st.tone}">${esc(st.label)}</span>${elapsed ? `<span>· ${esc(elapsed)}</span>` : ''}`;
  }

  let timer = null, dead = false;
  const poll = async () => {
    await fetchChild(parentId, agentId);
    if (dead) return;
    const parent = sessions.get(parentId);
    const child = parent && childrenOf(parent).find((c) => c.id === agentId);
    timer = setTimeout(poll, child && childActive(child) ? 1500 : 10000);
  };
  poll();
  render();
  pin.pin();
  return {
    update(ids) { if (ids.has('*') || ids.has(parentId)) render(); },
    destroy() { dead = true; clearTimeout(timer); ro.disconnect(); },
  };
}
