// Projects (native ui/projects.rs): the hub's shared project registry,
// pinned first. Pinning writes config.yaml (operator), read back to confirm.
import { can, isOperator } from '../bus.js';
import { loadConfig } from '../store.js';
import { esc, ic } from '../util.js';
import { notice } from '../ui.js';
import { projectList, setPinned } from '../projects.js';

export function mount(root, ctx) {
  root.innerHTML = `<div class="screen page-screen">
    <div class="pagehead"><button class="ibtn" data-back aria-label="Back">${ic('chevron-left', 's20')}</button><span class="grow"></span></div>
    <div class="page scrolly">
      <div class="ptitle"><span class="overline">Workspace</span><h1>Projects</h1><p>Folders on the hub’s machine your agents work in. Pinned projects come first everywhere.</p></div>
      <div data-list></div>
    </div>
  </div>`;
  const $ = (s) => root.querySelector(s);
  $('[data-back]').onclick = () => ctx.back();
  const busy = new Set();
  $('[data-list]').onclick = async (e) => {
    const b = e.target.closest('button');
    if (!b) return;
    if (b.dataset.new) ctx.go('#/new?cwd=' + encodeURIComponent(b.dataset.new));
    else if (b.dataset.pin) {
      const path = b.dataset.pin, on = b.dataset.on !== '1';
      busy.add(path); update();
      const err = await setPinned(path, on);
      busy.delete(path);
      notice(err || (on ? 'Pinned' : 'Unpinned'), err ? 'error' : 'success');
      update();
    }
  };
  const canPin = () => can('config.save') && isOperator();
  function update() {
    const list = projectList();
    $('[data-list]').innerHTML = list.length ? `<div class="group">${list.map((p) => `<div class="prow">
        <span class="ptag sm" style="color:${p.tint};background:color-mix(in srgb, ${p.tint} 22%, transparent)">${esc(p.tag)}</span>
        <div class="tx"><div class="n">${esc(p.name)}${p.running ? ` <span class="badge neutral">${p.running} running</span>` : ''}</div><div class="m mono">${esc(p.path)}</div></div>
        <button class="ibtn${p.pinned ? ' t-warning' : ''}" data-pin="${esc(p.path)}" data-on="${p.pinned ? 1 : 0}" aria-pressed="${p.pinned}" aria-label="${p.pinned ? 'Unpin' : 'Pin'} ${esc(p.name)}"${canPin() && !busy.has(p.path) ? '' : ' disabled'}>${ic('star', `s18${p.pinned ? ' filled' : ''}`)}</button>
        <button class="ibtn" data-new="${esc(p.path)}" aria-label="New agent in ${esc(p.name)}"${can('agents.spawn') ? '' : ' disabled'}>${ic('plus', 's18')}</button>
      </div>`).join('')}</div>${canPin() ? '' : '<p class="muted small">Pinning changes the hub’s shared settings and needs an operator token.</p>'}`
      : `<div class="empty">${ic('folder', 's20 muted')}<b>No projects yet</b><p>Start an agent in a folder and it shows up here.</p></div>`;
  }
  loadConfig().then(update);
  update();
  return { update };
}
