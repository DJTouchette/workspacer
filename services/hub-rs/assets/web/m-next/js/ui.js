// Shared chrome: bottom sheets, confirmations, pickers and island notices.
// Native has no toasts; a notice drops out of the top of the screen (the
// title island on a phone) and leaves on its own.
import { esc, ic } from './util.js';

const layer = () => document.getElementById('overlay');

let sheetSeq = 0;
let openSheet = null;
/** Present a bottom sheet. `html` is its content (after the grab handle);
 *  `bind(root, close)` wires it. Returns close(). Only one sheet at a time. */
export function sheet(html, { bind, onClose, cls = '', label = 'Sheet' } = {}) {
  closeSheet();
  const id = ++sheetSeq;
  const root = layer();
  root.innerHTML = `<div class="scrim" data-scrim></div>
    <div class="sheet ${cls}" role="dialog" aria-modal="true" aria-label="${esc(label)}"><div class="grab"></div>${html}</div>`;
  root.hidden = false;
  requestAnimationFrame(() => root.classList.add('show'));
  const el = root.querySelector('.sheet');
  const close = () => {
    if (!openSheet || openSheet.id !== id) return;
    openSheet = null;
    root.classList.remove('show');
    root.hidden = true;
    root.innerHTML = '';
    if (onClose) onClose();
  };
  openSheet = { id, close, el };
  root.querySelector('[data-scrim]').onclick = close;
  for (const b of el.querySelectorAll('[data-close]')) b.onclick = close;
  if (bind) bind(el, close);
  return close;
}
export function closeSheet() { if (openSheet) openSheet.close(); }
export const sheetOpen = () => !!openSheet;
/** Re-render the open sheet's body in place (keeps it open). */
export function sheetRoot() { return openSheet ? openSheet.el : null; }

/** An in-app confirmation: resolves true on confirm. */
export function ask({ title, body = '', confirm = 'Continue', cancel = 'Cancel', danger = false }) {
  return new Promise((resolve) => {
    let answered = false;
    sheet(`<div class="sh"><h3>${esc(title)}</h3></div>
      ${body ? `<p class="sheet-body">${esc(body)}</p>` : ''}
      <div class="row2"><button class="btn block" data-no>${esc(cancel)}</button>
      <button class="btn block ${danger ? 'danger' : 'primary'}" data-yes>${esc(confirm)}</button></div>`, {
      label: title,
      bind(el, close) {
        el.querySelector('[data-yes]').onclick = () => { answered = true; close(); resolve(true); };
        el.querySelector('[data-no]').onclick = close;
      },
      onClose: () => { if (!answered) resolve(false); },
    });
  });
}

/** A list picker. options: [{value, label, detail?, on?, disabled?}]. */
export function pick(title, options, { footer = '' } = {}) {
  return new Promise((resolve) => {
    let chosen = false;
    const rows = options.map((o, i) => `<button class="mi${o.on ? ' on' : ''}" data-i="${i}"${o.disabled ? ' disabled' : ''}>
        <span class="tx">${esc(o.label)}${o.detail ? `<span class="s">${esc(o.detail)}</span>` : ''}</span>
        ${o.on ? ic('check', 's16 t-accent') : ''}</button>`).join('');
    sheet(`<div class="sh"><h3>${esc(title)}</h3><button class="ibtn" data-close aria-label="Close">${ic('x', 's18')}</button></div>
      <div class="group menu scrolly">${rows || '<div class="mi" disabled>Nothing to choose</div>'}</div>${footer}`, {
      label: title,
      bind(el, close) {
        for (const b of el.querySelectorAll('[data-i]')) {
          b.onclick = () => { chosen = true; close(); resolve(options[Number(b.dataset.i)].value); };
        }
      },
      onClose: () => { if (!chosen) resolve(undefined); },
    });
  });
}

let noticeTimer = null;
/** Drop a notice from the top. tone: '', 'error', 'warning', 'success'. */
export function notice(text, tone = '') {
  const el = document.getElementById('notice');
  if (!el) return;
  el.className = 'notice show' + (tone ? ' ' + tone : '');
  el.innerHTML = `${ic(tone === 'error' ? 'circle-x' : tone === 'warning' ? 'triangle-alert' : tone === 'success' ? 'circle-check' : 'info', 's16')}<span>${esc(text)}</span>`;
  el.setAttribute('role', tone === 'error' ? 'alert' : 'status');
  clearTimeout(noticeTimer);
  noticeTimer = setTimeout(() => el.classList.remove('show'), tone === 'error' ? 5000 : 3000);
  el.onclick = () => el.classList.remove('show');
}

/** Keep a scroll container pinned to the bottom while the user is there. */
export function stickToBottom(scroller) {
  let pinned = true;
  scroller.addEventListener('scroll', () => {
    pinned = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 80;
  }, { passive: true });
  return {
    get pinned() { return pinned; },
    pin() { pinned = true; scroller.scrollTop = scroller.scrollHeight; },
    after() { if (pinned) scroller.scrollTop = scroller.scrollHeight; },
  };
}
