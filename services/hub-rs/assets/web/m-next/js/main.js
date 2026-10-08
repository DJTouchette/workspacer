// /m-next entry: the router, the viewport plumbing ported from /m, and the
// wiring between the bus, the store and whichever screen is showing.
//
// Routes (hash, so the phone's back gesture works):
//   #/                    Sessions        #/new[?cwd=]   New agent
//   #/s/<id>              Chat            #/history      Session history
//   #/s/<id>/a/<agent>    Subagent view   #/projects  #/jobs  #/settings  #/briefs
import { bus, connect, on as onBus } from './bus.js';
import { onChange, changed } from './store.js';
import { apply as applyTheme } from './theme.js';
import { registerSW, onOpenAgent } from './push.js';
import { closeSheet } from './ui.js';
import * as sessionsView from './views/sessions.js';
import * as chatView from './views/chat.js';
import * as childView from './views/child.js';
import * as newView from './views/newagent.js';
import * as historyView from './views/history.js';
import * as projectsView from './views/projects.js';
import * as jobsView from './views/jobs.js';
import * as settingsView from './views/settings.js';
import * as briefsView from './views/briefs.js';
import { tokenGate, machineGate } from './views/gate.js';

applyTheme();
const app = document.getElementById('app');
let view = null;
let viewKey = '';
let depth = 0;   // in-app navigations this session, so Back never leaves the app

function parse() {
  const raw = location.hash.replace(/^#/, '') || '/';
  const [path, query = ''] = raw.split('?');
  const parts = path.split('/').filter(Boolean).map(decodeURIComponent);
  return { parts, params: new URLSearchParams(query), key: raw };
}

const ctx = {
  go(hash, replace = false) {
    closeSheet();
    if (replace) { location.replace(location.pathname + location.search + hash); return; }
    depth++;
    location.hash = hash;
  },
  back() {
    closeSheet();
    if (depth > 0) { depth--; history.back(); }
    else location.replace(location.pathname + location.search + '#/');
  },
};

function route() {
  const overlay = document.getElementById('overlay');
  if (overlay && !overlay.hidden && overlay.classList.contains('menu')) { overlay.hidden = true; overlay.innerHTML = ''; overlay.classList.remove('show', 'menu'); }
  if (bus.machinePaused) { destroy(); viewKey = 'machine'; machineGate(app); return; }
  if (!bus.token) { destroy(); viewKey = 'gate'; tokenGate(app); return; }
  const { parts, params, key } = parse();
  if (key === viewKey && view) return;
  closeSheet();
  destroy();
  viewKey = key;
  const [head, id, sub, agent] = parts;
  if (head === 's' && id && sub === 'a' && agent) view = childView.mount(app, ctx, id, agent);
  else if (head === 's' && id) view = chatView.mount(app, ctx, id);
  else if (head === 'new') view = newView.mount(app, ctx, params);
  else if (head === 'history') view = historyView.mount(app, ctx);
  else if (head === 'projects') view = projectsView.mount(app, ctx);
  else if (head === 'jobs') view = jobsView.mount(app, ctx);
  else if (head === 'settings') view = settingsView.mount(app, ctx);
  else if (head === 'briefs') view = briefsView.mount(app, ctx);
  else view = sessionsView.mount(app, ctx);
  app.querySelector('.scrolly')?.scrollTo?.(0, 0);
}
function destroy() {
  if (view && view.destroy) view.destroy();
  view = null;
}

window.addEventListener('hashchange', route);
// Escape closes whatever sits on top (a sheet, or the expanded island).
document.addEventListener('keydown', (e) => {
  if (e.key !== 'Escape') return;
  const overlay = document.getElementById('overlay');
  if (!overlay || overlay.hidden) return;
  e.preventDefault();
  if (overlay.classList.contains('menu')) overlay.querySelector('[data-x]')?.click();
  else closeSheet();
});
onChange((ids) => { if (view && view.update) view.update(ids); });
onBus('unauthorized', route);
onBus('paused', route);
onBus('state', () => { if (!view) route(); });
onBus('hello', () => changed());

// A notification tap (or /m-next/?agent=<id>) names a session to open.
const qs = new URLSearchParams(location.search);
let pendingAgent = qs.get('agent') || '';
function openPending() {
  if (!pendingAgent || !bus.token) return;
  const id = pendingAgent; pendingAgent = '';
  ctx.go('#/s/' + encodeURIComponent(id));
}
onOpenAgent((id) => { pendingAgent = id; openPending(); });
if (pendingAgent) {
  // Drop ?agent= so a reload does not reopen it; keep the token query.
  qs.delete('agent');
  history.replaceState(null, '', location.pathname + (qs.toString() ? '?' + qs : '') + location.hash);
}

// ── viewport (ported from /m) ─────────────────────────────────────────────
// --vh is the VISUAL viewport height: excludes stale standalone chrome and the
// software keyboard; when iOS scrolls the window to reveal a focused input we
// follow that offset instead of leaving a keyboard-shaped void. The iOS 26
// standalone "dead strip" below the paintable area zeroes the bottom safe area.
const standalone = navigator.standalone === true || (window.matchMedia && matchMedia('(display-mode: standalone)').matches);
let lvhCache = { key: '', v: 0 };
const probeLvh = () => {
  const key = innerWidth + 'x' + innerHeight;
  if (lvhCache.key !== key) {
    const p = document.createElement('div');
    p.style.cssText = 'position:fixed;left:-9999px;top:0;width:10px;height:100lvh;visibility:hidden;';
    document.body.appendChild(p);
    lvhCache = { key, v: p.offsetHeight };
    p.remove();
  }
  return lvhCache.v;
};
let maxVH = innerHeight;
function applyViewport() {
  const vv = window.visualViewport;
  maxVH = Math.max(maxVH, innerHeight);
  const h = Math.round(vv ? vv.height : innerHeight);
  const off = vv ? Math.max(0, Math.round(vv.offsetTop)) : 0;
  const deadStrip = standalone ? Math.max(0, probeLvh() - innerHeight) : 0;
  document.body.classList.toggle('dead-strip', deadStrip > 20);
  document.documentElement.style.setProperty('--vh', h + 'px');
  app.style.transform = off ? `translateY(${off}px)` : '';
  document.body.classList.toggle('kb', innerHeight - h > 80);
}
// WebKit standalone bug: the first keyboard can leave the viewport ~80pt short
// until a synchronous reflow of the full-height shell makes it re-measure.
function healViewport() {
  if (!standalone) return;
  const vv = window.visualViewport;
  if (vv && innerHeight - vv.height > 80) return;
  if (maxVH - innerHeight <= 4) return;
  const scrollers = [...app.querySelectorAll('.scrolly')].map((el) => [el, el.scrollTop]);
  app.style.display = 'none';
  void app.offsetHeight;
  app.style.display = '';
  for (const [el, top] of scrollers) el.scrollTop = top;
  applyViewport();
}
document.addEventListener('focusout', () => setTimeout(healViewport, 140));
addEventListener('resize', applyViewport);
addEventListener('orientationchange', applyViewport);
if (window.visualViewport) {
  visualViewport.addEventListener('resize', applyViewport);
  visualViewport.addEventListener('scroll', applyViewport);
}
const remeasure = () => { applyViewport(); setTimeout(applyViewport, 350); setTimeout(healViewport, 400); };
document.addEventListener('visibilitychange', () => { if (document.visibilityState === 'visible') remeasure(); });
addEventListener('pageshow', remeasure);
applyViewport();

registerSW();
route();
connect();
onBus('open', openPending);
