/**
 * The host-authored script that runs INSIDE a response card's frame.
 *
 * This is the whole of "dynamic cards". A card gets local interaction —
 * disclosure, filtering, sorting, and an auto-sized frame — by opting into
 * documented `data-wks-*` attributes that this fixed runtime interprets. It
 * never runs model-authored code: the shell's CSP is `script-src 'nonce-…'`
 * and this string is the only thing that carries the nonce, so generated markup
 * has no way to supply a script or an event handler even if the sanitizer one
 * day misses one.
 *
 * The attribute vocabulary here is the contract the skill teaches, so a change
 * to either has to land in both (`main/services/responseCardSkill.ts`).
 *
 * Everything else `<details>`/`<summary>` already does natively, with no script
 * at all — the runtime deliberately adds nothing there.
 */

/** Message shape the frame posts out. The host validates `event.source`
 *  identity (a sandboxed srcDoc frame's origin is the literal string "null" for
 *  EVERY such frame, so origin cannot tell two cards apart) and clamps the
 *  value; nothing else crosses the boundary in either direction. */
export const CARD_HEIGHT_MESSAGE = 'wks-card-height';

/** Frame height bounds, in CSS px. Below the floor a one-line card looks
 *  broken; above the ceiling a card would push the rest of the transcript off
 *  screen, so it scrolls inside itself instead. */
export const CARD_MIN_HEIGHT = 40;
export const CARD_MAX_HEIGHT = 560;

export const CARD_RUNTIME_JS = `
(function () {
  'use strict';
  var HEIGHT_MSG = ${JSON.stringify(CARD_HEIGHT_MESSAGE)};

  // ── auto-height ───────────────────────────────────────────────────────────
  // A sandboxed frame cannot size itself, and the host will not trust a number
  // it did not bound, so this reports and the host clamps.
  var last = -1;
  function report() {
    var h = Math.ceil(document.body.getBoundingClientRect().height);
    if (h === last) return;
    last = h;
    try { parent.postMessage({ type: HEIGHT_MSG, height: h }, '*'); } catch (e) {}
  }
  if (typeof ResizeObserver === 'function') {
    new ResizeObserver(report).observe(document.body);
  }
  window.addEventListener('load', report);
  report();

  // ── data-wks-filter: an input that hides non-matching rows ────────────────
  // <input data-wks-filter="tbl"> filters, inside #tbl, every element carrying
  // data-wks-filter-item by case-insensitive substring of its text.
  function applyFilter(input) {
    var scope = document.getElementById(input.getAttribute('data-wks-filter') || '');
    if (!scope) return;
    var needle = String(input.value || '').toLowerCase().trim();
    var items = scope.querySelectorAll('[data-wks-filter-item]');
    var shown = 0;
    for (var i = 0; i < items.length; i++) {
      var hit = !needle || items[i].textContent.toLowerCase().indexOf(needle) !== -1;
      items[i].hidden = !hit;
      if (hit) shown++;
    }
    var count = scope.querySelector('[data-wks-filter-count]');
    if (count) count.textContent = String(shown);
    report();
  }
  document.addEventListener('input', function (e) {
    var t = e.target;
    if (t && t.hasAttribute && t.hasAttribute('data-wks-filter')) applyFilter(t);
  });

  // ── data-wks-sort: click a <th> to sort its table ─────────────────────────
  // <th data-wks-sort="text"> or "number". Sorts the rows of the th's table's
  // first <tbody>; a second click reverses.
  function cellValue(row, index, mode) {
    var cell = row.children[index];
    var text = cell ? cell.textContent.trim() : '';
    if (mode !== 'number') return text.toLowerCase();
    var n = parseFloat(text.replace(/[^0-9eE+.-]/g, ''));
    return isNaN(n) ? -Infinity : n;
  }
  document.querySelectorAll('th[data-wks-sort]').forEach(function (th) {
    th.tabIndex = 0;
    th.setAttribute('aria-sort', 'none');
    th.addEventListener('keydown', function (e) {
      if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); th.click(); }
    });
  });
  document.addEventListener('click', function (e) {
    var th = e.target && e.target.closest ? e.target.closest('[data-wks-sort]') : null;
    if (!th) return;
    var table = th.closest('table');
    var body = table && table.tBodies[0];
    if (!body) return;
    var index = Array.prototype.indexOf.call(th.parentNode.children, th);
    var mode = th.getAttribute('data-wks-sort') === 'number' ? 'number' : 'text';
    var dir = th.getAttribute('data-wks-sort-dir') === 'asc' ? -1 : 1;
    var rows = Array.prototype.slice.call(body.rows);
    rows.sort(function (a, b) {
      var x = cellValue(a, index, mode), y = cellValue(b, index, mode);
      return x < y ? -dir : x > y ? dir : 0;
    });
    for (var i = 0; i < rows.length; i++) body.appendChild(rows[i]);
    var heads = table.querySelectorAll('[data-wks-sort]');
    for (var j = 0; j < heads.length; j++) heads[j].removeAttribute('data-wks-sort-dir');
    th.setAttribute('data-wks-sort-dir', dir === 1 ? 'asc' : 'desc');
    th.setAttribute('aria-sort', dir === 1 ? 'ascending' : 'descending');
    report();
  });

  // <details> toggling changes height with no script of its own.
  document.addEventListener('toggle', report, true);
})();
`;
