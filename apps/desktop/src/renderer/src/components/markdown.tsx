/**
 * Lightweight markdown renderer — no external deps. Handles the subset that
 * shows up in Claude output and library prompts/skills: headings, fenced code,
 * lists, blockquotes, rules, bold/italic/inline-code/links.
 *
 * Extracted from ClaudePane; also powers the Library editor's preview.
 */
import React from 'react';
import { claudeColors as colors } from './claude-shared';
import { langForInfo } from '../lib/diff/highlight';
import { useHighlight, renderLine } from './claude/highlight';

/** Fenced code block with lazy syntax highlighting (plain text until the
 *  grammar loads / when the language is unknown). */
const CodeBlock: React.FC<{ code: string; info?: string }> = ({ code, info }) => {
  const lang = info ? langForInfo(info) : null;
  const tokens = useHighlight(code, lang);
  const lines = code.split('\n');
  return (
    <div style={{ margin: '6px 0' }}>
      {info && (
        <div style={{
          fontSize: '0.6rem',
          color: colors.muted,
          backgroundColor: 'rgba(255,255,255,0.04)',
          padding: '2px 10px',
          borderRadius: '6px 6px 0 0',
          borderBottom: `1px solid ${colors.border}`,
          fontFamily: 'var(--claude-mono-font, monospace)',
        }}>
          {info}
        </div>
      )}
      <pre style={{
        margin: 0,
        padding: '10px 12px',
        backgroundColor: 'rgba(0, 0, 0, 0.3)',
        borderRadius: info ? '0 0 6px 6px' : '6px',
        fontSize: '0.75rem',
        lineHeight: 1.5,
        color: 'rgb(190, 200, 220)',
        fontFamily: 'var(--claude-mono-font, monospace)',
        overflowX: 'auto',
        whiteSpace: 'pre',
        border: `1px solid ${colors.border}`,
        borderTop: info ? 'none' : undefined,
      }}>
        {lines.map((ln, i) => (
          <React.Fragment key={i}>
            {renderLine(tokens?.[i], ln)}
            {i < lines.length - 1 ? '\n' : ''}
          </React.Fragment>
        ))}
      </pre>
    </div>
  );
};

/** Render inline markdown: **bold**, *italic*, `code`, [links](url) */
export function renderInlineMarkdown(text: string): React.ReactNode[] {
  const nodes: React.ReactNode[] = [];
  const regex = /(\*\*(.+?)\*\*|\*(.+?)\*|`([^`]+)`|\[([^\]]+)\]\(([^)]+)\))/g;
  let lastIndex = 0;
  let match: RegExpExecArray | null;
  let key = 0;

  while ((match = regex.exec(text)) !== null) {
    if (match.index > lastIndex) {
      nodes.push(text.slice(lastIndex, match.index));
    }

    if (match[2]) {
      nodes.push(<strong key={key++} style={{ color: colors.textBright, fontWeight: 700 }}>{match[2]}</strong>);
    } else if (match[3]) {
      nodes.push(<em key={key++} style={{ color: 'rgb(210, 210, 230)', fontStyle: 'italic' }}>{match[3]}</em>);
    } else if (match[4]) {
      nodes.push(
        <code key={key++} style={{
          backgroundColor: 'rgba(255, 255, 255, 0.07)',
          padding: '1px 5px',
          borderRadius: 3,
          fontSize: '0.9em',
          fontFamily: 'var(--claude-mono-font, monospace)',
          color: 'rgb(180, 210, 255)',
        }}>
          {match[4]}
        </code>
      );
    } else if (match[5] && match[6]) {
      nodes.push(
        <span key={key++} style={{ color: colors.accent, textDecoration: 'underline', cursor: 'default' }} title={match[6]}>
          {match[5]}
        </span>
      );
    }

    lastIndex = match.index + match[0].length;
  }

  if (lastIndex < text.length) {
    nodes.push(text.slice(lastIndex));
  }

  return nodes.length > 0 ? nodes : [text];
}

// Module-level LRU cache for parsed markdown — bounded to 300 entries so
// identical content (completed turns) never re-parses across re-renders.
const MD_CACHE_MAX = 300;
const mdCache = new Map<string, React.ReactNode[]>();

/** Clear the module-level markdown cache (call on session switch). */
export function clearMdCache(): void {
  mdCache.clear();
}

function mdCachePut(key: string, value: React.ReactNode[]): React.ReactNode[] {
  if (mdCache.size >= MD_CACHE_MAX) {
    // Evict oldest entry (Map iteration order = insertion order)
    mdCache.delete(mdCache.keys().next().value!);
  }
  mdCache.set(key, value);
  return value;
}

// ---- Tables -------------------------------------------------------------
type ColAlign = 'left' | 'center' | 'right';

/** Split a pipe-delimited row into trimmed cells, dropping the optional
 *  leading/trailing border pipes. */
function splitPipeCells(line: string): string[] {
  let t = line.trim();
  if (t.startsWith('|')) t = t.slice(1);
  if (t.endsWith('|')) t = t.slice(0, -1);
  return t.split('|').map(c => c.trim());
}

/** A GFM delimiter row: every cell is dashes with optional leading/trailing
 *  colon (alignment), e.g. `|:---|---:|`. */
function isPipeSeparator(line: string): boolean {
  const t = line.trim();
  if (!t.includes('-')) return false;
  const cells = splitPipeCells(line);
  return cells.length > 0 && cells.every(c => /^:?-{1,}:?$/.test(c));
}

function pipeAligns(sepLine: string): ColAlign[] {
  return splitPipeCells(sepLine).map(c => {
    const l = c.startsWith(':'), r = c.endsWith(':');
    return l && r ? 'center' : r ? 'right' : 'left';
  });
}

const VBAR_RE = /[|│┃]/; // | │ ┃
const BOX_JUNCTION_RE = /[┌┬┐├┼┤└┴┘]/; // ┌┬┐├┼┤└┴┘

/** A horizontal border of a "drawn" table: `+---+---+` or box-drawing
 *  `┌──┬──┐` / `├──┼──┤` / `└──┴──┘`. */
function isDrawnBorder(line: string): boolean {
  const t = line.trim();
  if (!t) return false;
  if (/^[+\-=\s]+$/.test(t) && t.includes('+') && /[-=]/.test(t)) return true;
  if (/^[─-╿\s]+$/.test(t) && BOX_JUNCTION_RE.test(t)) return true;
  return false;
}

function isDrawnContentRow(line: string): boolean {
  const t = line.trim();
  return VBAR_RE.test(t) && (t.startsWith('|') || t.startsWith('│') || t.startsWith('┃'));
}

function splitDrawnCells(line: string): string[] {
  const cells = line.trim().split(VBAR_RE).map(c => c.trim());
  if (cells.length && cells[0] === '') cells.shift();
  if (cells.length && cells[cells.length - 1] === '') cells.pop();
  return cells;
}

interface DrawnTable { header: string[] | null; rows: string[][]; end: number; }

/** Parse a drawn (border-and-bar) table starting at `start`. Returns null if
 *  `start` isn't a border row or the lines don't form a coherent table. The
 *  rows between the first two borders are treated as the header; everything
 *  after the second border is the body. With only top/bottom borders there's
 *  no header. */
function parseDrawnTable(lines: string[], start: number): DrawnTable | null {
  if (!isDrawnBorder(lines[start])) return null;
  const borders: number[] = [];
  const content: { idx: number; cells: string[] }[] = [];
  let i = start;
  while (i < lines.length) {
    const ln = lines[i];
    if (ln.trim() === '') break;
    if (isDrawnBorder(ln)) { borders.push(i); i++; continue; }
    if (isDrawnContentRow(ln)) { content.push({ idx: i, cells: splitDrawnCells(ln) }); i++; continue; }
    break;
  }
  if (content.length === 0 || borders.length < 2) return null;
  const secondBorder = borders[1];
  const before = content.filter(c => c.idx < secondBorder).map(c => c.cells);
  const after = content.filter(c => c.idx > secondBorder).map(c => c.cells);
  let header: string[] | null = null;
  let rows: string[][];
  if (before.length > 0 && after.length > 0) {
    header = before[0];
    rows = before.length > 1 ? [...before.slice(1), ...after] : after;
  } else {
    rows = content.map(c => c.cells);
  }
  return { header, rows, end: i };
}

/** If a fenced block's body is nothing but a drawn table, return it. Used to
 *  re-render CLI tool output (which arrives fenced) as a real table. */
function drawnTableFromBlock(codeLines: string[]): DrawnTable | null {
  let f = 0;
  while (f < codeLines.length && codeLines[f].trim() === '') f++;
  if (f >= codeLines.length) return null;
  const tbl = parseDrawnTable(codeLines, f);
  if (!tbl) return null;
  let rest = tbl.end;
  while (rest < codeLines.length && codeLines[rest].trim() === '') rest++;
  return rest >= codeLines.length ? tbl : null;
}

function renderTable(key: number, header: string[] | null, rows: string[][], aligns?: ColAlign[]): React.ReactNode {
  const colCount = Math.max(header?.length ?? 0, ...rows.map(r => r.length), 1);
  const cols = Array.from({ length: colCount }, (_, n) => n);
  const align = (ci: number): ColAlign => aligns?.[ci] ?? 'left';
  return (
    <div key={key} style={{ margin: '8px 0', overflowX: 'auto' }}>
      <table style={{
        borderCollapse: 'collapse',
        fontSize: '0.75rem',
        lineHeight: 1.5,
        minWidth: '100%',
      }}>
        {header && (
          <thead>
            <tr>
              {cols.map(ci => (
                <th key={ci} style={{
                  textAlign: align(ci),
                  fontWeight: 700,
                  color: colors.textBright,
                  padding: '4px 10px',
                  borderBottom: `1px solid ${colors.divider}`,
                  whiteSpace: 'nowrap',
                }}>
                  {renderInlineMarkdown(header[ci] ?? '')}
                </th>
              ))}
            </tr>
          </thead>
        )}
        <tbody>
          {rows.map((r, ri) => (
            <tr key={ri} style={{ backgroundColor: ri % 2 === 1 ? 'rgba(255,255,255,0.02)' : 'transparent' }}>
              {cols.map(ci => (
                <td key={ci} style={{
                  textAlign: align(ci),
                  padding: '3px 10px',
                  borderBottom: `1px solid ${colors.borderSubtle}`,
                  color: colors.text,
                  verticalAlign: 'top',
                }}>
                  {renderInlineMarkdown(r[ci] ?? '')}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/** Parse a markdown string into structured blocks */
export function parseMarkdownBlocks(text: string): React.ReactNode[] {
  if (!text) return [];
  const cached = mdCache.get(text);
  if (cached) return cached;

  const lines = text.split('\n');
  const blocks: React.ReactNode[] = [];
  let key = 0;
  let i = 0;

  while (i < lines.length) {
    const line = lines[i];

    // Fenced code block
    if (line.trimStart().startsWith('```')) {
      const lang = line.trimStart().slice(3).trim();
      const codeLines: string[] = [];
      i++;
      while (i < lines.length && !lines[i].trimStart().startsWith('```')) {
        codeLines.push(lines[i]);
        i++;
      }
      i++;
      // CLI tool output arrives fenced; if the whole block is a drawn table,
      // render it as a real table instead of preformatted text.
      const fenceTable = drawnTableFromBlock(codeLines);
      if (fenceTable) {
        blocks.push(renderTable(key++, fenceTable.header, fenceTable.rows));
        continue;
      }
      blocks.push(<CodeBlock key={key++} code={codeLines.join('\n')} info={lang} />);
      continue;
    }

    // Pipe table: a header row of cells over a |---|---| delimiter row. Require
    // the header and delimiter to have the same number of cells (GFM rule), so
    // a paragraph line with a stray `|` followed by a `---` setext/HR doesn't
    // get mistaken for a table.
    if (
      line.includes('|') &&
      !isPipeSeparator(line) &&
      i + 1 < lines.length &&
      isPipeSeparator(lines[i + 1])
    ) {
      const header = splitPipeCells(line);
      const aligns = pipeAligns(lines[i + 1]);
      if (header.length >= 1 && header.length === aligns.length) {
        i += 2;
        const rows: string[][] = [];
        while (
          i < lines.length &&
          lines[i].trim() !== '' &&
          lines[i].includes('|') &&
          !isPipeSeparator(lines[i])
        ) {
          rows.push(splitPipeCells(lines[i]));
          i++;
        }
        blocks.push(renderTable(key++, header, rows, aligns));
        continue;
      }
    }

    // Drawn ASCII / box-drawing table: +---+ or ┌──┬──┐ borders with | / │ cells
    if (isDrawnBorder(line)) {
      const drawn = parseDrawnTable(lines, i);
      if (drawn) {
        blocks.push(renderTable(key++, drawn.header, drawn.rows));
        i = drawn.end;
        continue;
      }
    }

    // Heading
    const headingMatch = line.match(/^(#{1,4})\s+(.+)$/);
    if (headingMatch) {
      const level = headingMatch[1].length;
      const sizes: Record<number, string> = { 1: '0.95rem', 2: '0.88rem', 3: '0.82rem', 4: '0.78rem' };
      blocks.push(
        <div key={key++} style={{
          fontSize: sizes[level] ?? '0.78rem',
          fontWeight: 700,
          color: colors.textBright,
          margin: `${level === 1 ? 12 : 8}px 0 4px 0`,
          paddingBottom: level <= 2 ? 4 : 0,
          borderBottom: level <= 2 ? `1px solid ${colors.divider}` : 'none',
        }}>
          {renderInlineMarkdown(headingMatch[2])}
        </div>
      );
      i++;
      continue;
    }

    // Unordered list
    if (/^\s*[-*]\s+/.test(line)) {
      const listItems: { indent: number; content: string }[] = [];
      while (i < lines.length && /^\s*[-*]\s+/.test(lines[i])) {
        const m = lines[i].match(/^(\s*)[-*]\s+(.+)$/);
        if (m) listItems.push({ indent: m[1].length, content: m[2] });
        i++;
      }
      blocks.push(
        <div key={key++} style={{ margin: '4px 0' }}>
          {listItems.map((item, idx) => (
            <div key={idx} style={{ display: 'flex', gap: 6, paddingLeft: Math.min(item.indent, 12) + 4, marginBottom: 2 }}>
              <span style={{ color: colors.accent, flexShrink: 0, lineHeight: 1.6 }}>{'•'}</span>
              <span style={{ lineHeight: 1.6 }}>{renderInlineMarkdown(item.content)}</span>
            </div>
          ))}
        </div>
      );
      continue;
    }

    // Ordered list
    if (/^\s*\d+[.)]\s+/.test(line)) {
      const listItems: { num: string; content: string }[] = [];
      while (i < lines.length && /^\s*\d+[.)]\s+/.test(lines[i])) {
        const m = lines[i].match(/^\s*(\d+)[.)]\s+(.+)$/);
        if (m) listItems.push({ num: m[1], content: m[2] });
        i++;
      }
      blocks.push(
        <div key={key++} style={{ margin: '4px 0' }}>
          {listItems.map((item, idx) => (
            <div key={idx} style={{ display: 'flex', gap: 6, paddingLeft: 4, marginBottom: 2 }}>
              <span style={{ color: colors.muted, flexShrink: 0, minWidth: 14, textAlign: 'right', lineHeight: 1.6 }}>{item.num}.</span>
              <span style={{ lineHeight: 1.6 }}>{renderInlineMarkdown(item.content)}</span>
            </div>
          ))}
        </div>
      );
      continue;
    }

    // Horizontal rule
    if (/^[-*_]{3,}\s*$/.test(line.trim())) {
      blocks.push(<hr key={key++} style={{ border: 'none', borderTop: `1px solid ${colors.divider}`, margin: '8px 0' }} />);
      i++;
      continue;
    }

    // Blockquote
    if (line.startsWith('>')) {
      const quoteLines: string[] = [];
      while (i < lines.length && lines[i].startsWith('>')) {
        quoteLines.push(lines[i].replace(/^>\s?/, ''));
        i++;
      }
      blocks.push(
        <div key={key++} style={{
          borderLeft: `2px solid ${colors.muted}`,
          paddingLeft: 10,
          margin: '4px 0',
          color: 'rgb(160, 165, 185)',
          fontStyle: 'italic',
          lineHeight: 1.6,
        }}>
          {renderInlineMarkdown(quoteLines.join(' '))}
        </div>
      );
      continue;
    }

    // Empty line
    if (line.trim() === '') {
      blocks.push(<div key={key++} style={{ height: 4 }} />);
      i++;
      continue;
    }

    // Regular paragraph
    const paraLines: string[] = [];
    while (
      i < lines.length &&
      lines[i].trim() !== '' &&
      !lines[i].trimStart().startsWith('```') &&
      !lines[i].match(/^#{1,4}\s+/) &&
      !/^\s*[-*]\s+/.test(lines[i]) &&
      !/^\s*\d+[.)]\s+/.test(lines[i]) &&
      !lines[i].startsWith('>') &&
      !/^[-*_]{3,}\s*$/.test(lines[i].trim()) &&
      !(lines[i].includes('|') && i + 1 < lines.length && isPipeSeparator(lines[i + 1])) &&
      !isDrawnBorder(lines[i])
    ) {
      paraLines.push(lines[i]);
      i++;
    }

    if (paraLines.length > 0) {
      blocks.push(
        <p key={key++} style={{ margin: '3px 0', lineHeight: 1.6, wordBreak: 'break-word' }}>
          {renderInlineMarkdown(paraLines.join('\n'))}
        </p>
      );
    }
  }

  return mdCachePut(text, blocks);
}

/** Convenience component wrapper around parseMarkdownBlocks. */
export const Markdown: React.FC<{ text: string }> = ({ text }) => <>{parseMarkdownBlocks(text)}</>;
