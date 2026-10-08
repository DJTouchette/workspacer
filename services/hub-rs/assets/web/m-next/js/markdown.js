// Assistant text → HTML, the way native's transcript renders it: markdown with
// headings (hairline under h2), accent round bullets, tables, a blockquote
// rail, inline code in accent, and fenced code with a language header and
// light syntax highlighting. Response cards (```wks-html-card) and worker
// results (```wks-result) are split out first, exactly as native's
// `assistant_blocks` does.
//
// Everything is escaped before it reaches the DOM; only tags emitted here are
// live. Untrusted agent output runs on the phone's CPU, so the inline pass is
// length-bounded and the highlighter is a single linear scan.
import { esc } from './util.js';

const safeUrl = (u) => (/^(https?:|mailto:)/i.test(String(u || '').trim()) ? String(u).trim() : null);
const MAX_INLINE = 8192;

export function renderInline(text, depth = 0) {
  if (text.length > MAX_INLINE) return esc(text);
  const re = /(\*\*([\s\S]+?)\*\*|__([^_\n]+?)__|(?<![\w*])\*([^*\n]+?)\*(?!\w)|(?<![\w_])_([^_\n]+?)_(?!\w)|~~([^~\n]+?)~~|`([^`]+)`|\[([^\]\n]{1,512})\]\(([^)\s]{1,2048})\)|(https?:\/\/[^\s<>()]{3,2048}[^\s<>().,;:!?'"]))/g;
  let out = '', last = 0, m;
  const inner = (s) => (depth < 4 ? renderInline(s, depth + 1) : esc(s));
  while ((m = re.exec(text))) {
    out += esc(text.slice(last, m.index));
    if (m[2] !== undefined) out += `<strong>${inner(m[2])}</strong>`;
    else if (m[3] !== undefined) out += `<strong>${inner(m[3])}</strong>`;
    else if (m[4] !== undefined) out += `<em>${inner(m[4])}</em>`;
    else if (m[5] !== undefined) out += `<em>${inner(m[5])}</em>`;
    else if (m[6] !== undefined) out += `<del>${inner(m[6])}</del>`;
    else if (m[7] !== undefined) out += `<code>${esc(m[7])}</code>`;
    else if (m[8] !== undefined) {
      const url = safeUrl(m[9]);
      out += url ? `<a href="${esc(url)}" target="_blank" rel="noopener noreferrer">${inner(m[8])}</a>` : inner(m[8]);
    } else {
      out += `<a href="${esc(m[10])}" target="_blank" rel="noopener noreferrer">${esc(m[10])}</a>`;
    }
    last = re.lastIndex;
  }
  return out + esc(text.slice(last));
}

// ── syntax highlighting ──────────────────────────────────────────────────
const KW = {
  c: 'if else for while do return break continue switch case default function fn let const var mut pub use mod impl struct enum trait type interface class extends implements new this self super static async await yield import export from as in of match where loop try catch finally throw throws package func go defer chan select range map nil null true false None True False undefined void public private protected readonly final override virtual abstract sealed namespace using typeof instanceof delete crate dyn ref move unsafe extern',
  py: 'def class return if elif else for while in not and or is import from as with try except finally raise lambda yield pass break continue global nonlocal async await None True False self',
  sh: 'if then else elif fi for in do done while until case esac function return export local readonly set unset echo cd exit source sudo',
};
const FAMILY = {
  js: 'c', jsx: 'c', ts: 'c', tsx: 'c', javascript: 'c', typescript: 'c', mjs: 'c', cjs: 'c', json: 'c',
  rust: 'c', rs: 'c', go: 'c', java: 'c', kotlin: 'c', kt: 'c', swift: 'c', c: 'c', h: 'c', cpp: 'c', 'c++': 'c',
  cs: 'c', csharp: 'c', php: 'c', scala: 'c', dart: 'c', zig: 'c', css: 'c', scss: 'c',
  python: 'py', py: 'py', ruby: 'py', rb: 'py', toml: 'sh', yaml: 'sh', yml: 'sh', ini: 'sh',
  sh: 'sh', bash: 'sh', zsh: 'sh', shell: 'sh', console: 'sh', fish: 'sh', dockerfile: 'sh', make: 'sh', makefile: 'sh',
};
const kwSets = Object.fromEntries(Object.entries(KW).map(([k, v]) => [k, new Set(v.split(' '))]));

/** One linear pass: comments, strings, numbers, keywords, calls. */
export function highlight(code, lang) {
  const fam = FAMILY[String(lang || '').toLowerCase()];
  if (!fam || code.length > 60000) return esc(code);
  const kws = kwSets[fam];
  const hashComment = fam !== 'c';
  let out = '', i = 0;
  const n = code.length;
  const span = (cls, s) => `<span class="${cls}">${esc(s)}</span>`;
  while (i < n) {
    const c = code[i], d = code[i + 1];
    if ((fam === 'c' && c === '/' && d === '/') || (hashComment && c === '#')) {
      const j = code.indexOf('\n', i); const e = j < 0 ? n : j;
      out += span('com', code.slice(i, e)); i = e; continue;
    }
    if (fam === 'c' && c === '/' && d === '*') {
      const j = code.indexOf('*/', i + 2); const e = j < 0 ? n : j + 2;
      out += span('com', code.slice(i, e)); i = e; continue;
    }
    if (c === '"' || c === "'" || c === '`') {
      let j = i + 1;
      while (j < n && code[j] !== c && !(code[j] === '\n' && c !== '`')) j += code[j] === '\\' ? 2 : 1;
      out += span('str', code.slice(i, Math.min(n, j + 1))); i = Math.min(n, j + 1); continue;
    }
    if (/[0-9]/.test(c) && !/[\w$]/.test(code[i - 1] || '')) {
      const m = /^(0x[0-9a-f_]+|\d[\d_]*(\.\d+)?(e[+-]?\d+)?)/i.exec(code.slice(i, i + 40));
      if (m) { out += span('num', m[0]); i += m[0].length; continue; }
    }
    if (/[A-Za-z_$]/.test(c)) {
      let j = i + 1;
      while (j < n && /[\w$]/.test(code[j])) j++;
      const word = code.slice(i, j);
      if (kws.has(word)) out += span('kw', word);
      else if (code[j] === '(' || (code[j] === '!' && fam === 'c' && code[j + 1] === '(')) out += span('fn', word);
      else if (/^[A-Z][A-Za-z0-9]+$/.test(word) && fam === 'c') out += span('ty', word);
      else out += esc(word);
      i = j; continue;
    }
    out += esc(c); i++;
  }
  return out;
}

function codeBlock(lines, lang) {
  const code = lines.join('\n');
  return `<div class="codeblock"><div class="hd"><span>${esc(lang || 'text')}</span>` +
    `<button class="copy" data-copy-code aria-label="Copy code">Copy</button></div>` +
    `<pre><code>${highlight(code, lang)}</code></pre></div>`;
}

// ── tables ───────────────────────────────────────────────────────────────
const RE_TABLE_SEP = /^\s*\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)*\|?\s*$/;
const cells = (line) => {
  let s = line.trim();
  if (s.startsWith('|')) s = s.slice(1);
  if (s.endsWith('|') && !s.endsWith('\\|')) s = s.slice(0, -1);
  return s.split(/(?<!\\)\|/).map((c) => c.trim().replace(/\\\|/g, '|'));
};
function tableHtml(head, align, rows) {
  const a = (i) => (align[i] ? ` style="text-align:${align[i]}"` : '');
  return `<div class="tablewrap"><table><thead><tr>${head.map((c, i) => `<th${a(i)}>${renderInline(c)}</th>`).join('')}</tr></thead>` +
    `<tbody>${rows.map((r) => `<tr>${head.map((_, i) => `<td${a(i)}>${renderInline(r[i] || '')}</td>`).join('')}</tr>`).join('')}</tbody></table></div>`;
}

const RE_FENCE = /^\s{0,3}(`{3,}|~{3,})\s*([\w+#.-]*)/;
const RE_HEAD = /^(#{1,6})\s+(.+?)\s*#*\s*$/;
const RE_HR = /^\s*([-*_])(\s*\1){2,}\s*$/;
const RE_QUOTE = /^\s*>/;
const RE_LI = /^(\s*)([-*+]|\d+[.)])\s+(.*)$/;
const RE_TASK = /^\[([ xX])\]\s+/;
const isBlockStart = (l, next) => RE_FENCE.test(l) || RE_HEAD.test(l) || RE_HR.test(l) || RE_QUOTE.test(l) || RE_LI.test(l) ||
  (l.includes('|') && next !== undefined && RE_TABLE_SEP.test(next));

export function renderMarkdown(text) {
  const lines = String(text).replace(/\r\n?/g, '\n').split('\n');
  const out = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    const fence = RE_FENCE.exec(line);
    if (fence) {
      const marker = fence[1][0], width = fence[1].length;
      i++;
      const buf = [];
      while (i < lines.length) {
        const t = lines[i].trim();
        if (t.length >= width && [...t].every((ch) => ch === marker)) break;
        buf.push(lines[i]); i++;
      }
      if (i < lines.length) i++;
      out.push(codeBlock(buf, fence[2]));
      continue;
    }
    if (line.includes('|') && i + 1 < lines.length && RE_TABLE_SEP.test(lines[i + 1])) {
      const head = cells(line);
      const align = cells(lines[i + 1]).map((c) => (/^:-+:$/.test(c) ? 'center' : /-:$/.test(c) ? 'right' : ''));
      i += 2;
      const rows = [];
      while (i < lines.length && lines[i].includes('|') && lines[i].trim()) { rows.push(cells(lines[i])); i++; }
      out.push(tableHtml(head, align, rows));
      continue;
    }
    const h = RE_HEAD.exec(line);
    if (h) { const lv = Math.min(4, h[1].length); out.push(`<h${lv}>${renderInline(h[2])}</h${lv}>`); i++; continue; }
    if (RE_HR.test(line)) { out.push('<hr>'); i++; continue; }
    if (RE_QUOTE.test(line)) {
      const buf = [];
      while (i < lines.length && RE_QUOTE.test(lines[i])) { buf.push(lines[i].replace(/^\s*>\s?/, '')); i++; }
      out.push(`<blockquote>${renderMarkdown(buf.join('\n'))}</blockquote>`);
      continue;
    }
    if (RE_LI.test(line)) {
      out.push(listHtml(lines, i, (next) => { i = next; }));
      continue;
    }
    if (!line.trim()) { i++; continue; }
    const buf = [];
    while (i < lines.length && lines[i].trim() && !isBlockStart(lines[i], lines[i + 1])) { buf.push(lines[i]); i++; }
    if (!buf.length) { buf.push(lines[i]); i++; }
    out.push(`<p>${renderInline(buf.join('\n')).replace(/\n/g, '<br>')}</p>`);
  }
  return out.join('');
}

/** Lists with nesting by indent and task checkboxes. */
function listHtml(lines, start, done) {
  const first = RE_LI.exec(lines[start]);
  const indent = first[1].length;
  const ordered = /\d/.test(first[2]);
  const items = [];
  let i = start;
  while (i < lines.length) {
    const m = RE_LI.exec(lines[i]);
    if (!m) {
      // A continuation line indented under the item belongs to it.
      if (lines[i].trim() && /^\s+/.test(lines[i]) && items.length) { items[items.length - 1].body.push(lines[i].trim()); i++; continue; }
      break;
    }
    if (m[1].length < indent) break;
    if (m[1].length > indent) {
      const nested = [];
      const sub = listHtml(lines, i, (next) => { i = next; });
      nested.push(sub);
      if (items.length) items[items.length - 1].nested.push(...nested);
      continue;
    }
    items.push({ body: [m[3]], nested: [] });
    i++;
  }
  done(i);
  const li = items.map((it) => {
    let body = it.body.join(' ');
    const task = RE_TASK.exec(body);
    let cls = '';
    if (task) { cls = task[1] === ' ' ? ' class="task"' : ' class="task done"'; body = body.slice(task[0].length); }
    return `<li${cls}>${renderInline(body)}${it.nested.join('')}</li>`;
  }).join('');
  const startAttr = ordered && Number.parseInt(first[2], 10) > 1 ? ` start="${Number.parseInt(first[2], 10)}"` : '';
  return ordered ? `<ol${startAttr}>${li}</ol>` : `<ul>${li}</ul>`;
}

// ── assistant blocks: prose, response cards, worker results ─────────────
/** Split assistant text like native's `assistant_blocks`: only closed,
 *  top-level ```wks-html-card fences become cards; ```wks-result fences
 *  (any width ≥ 3, ` or ~) become results; a card inside another fence is
 *  ordinary code. Returns [{kind:'md'|'card'|'result', text}]. */
export function assistantBlocks(text) {
  const lines = String(text).replace(/\r\n?/g, '\n').split('\n');
  const blocks = [];
  let prose = [];
  const flush = () => { if (prose.length) { blocks.push({ kind: 'md', text: prose.join('\n') }); prose = []; } };
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    const trimmed = line.trimStart();
    const resultFence = /^(`{3,}|~{3,})\s*wks-result\s*$/.exec(trimmed);
    if (resultFence && line.length - trimmed.length <= 3) {
      const marker = resultFence[1][0], width = resultFence[1].length;
      let end = -1;
      for (let j = i + 1; j < lines.length; j++) {
        const t = lines[j].trim();
        if (t.length >= width && [...t].every((c) => c === marker)) { end = j; break; }
      }
      if (end > 0) { flush(); blocks.push({ kind: 'result', text: lines.slice(i + 1, end).join('\n') }); i = end + 1; continue; }
    }
    if (line === '```wks-html-card') {
      let end = -1;
      for (let j = i + 1; j < lines.length; j++) if (lines[j].trimEnd() === '```') { end = j; break; }
      if (end > 0) {
        flush();
        blocks.push({ kind: 'card', text: lines.slice(i + 1, end).join('\n') });
        i = end + 1;
        continue;
      }
    }
    const fence = /^(`{3,}|~{3,})/.exec(line);
    if (fence) {
      const marker = fence[1][0], width = fence[1].length;
      prose.push(line); i++;
      while (i < lines.length) {
        prose.push(lines[i]);
        const t = lines[i].trim(); i++;
        if (t.length >= width && [...t].every((c) => c === marker)) break;
      }
      continue;
    }
    prose.push(line); i++;
  }
  flush();
  return blocks;
}
