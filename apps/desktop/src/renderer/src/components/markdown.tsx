/**
 * Lightweight markdown renderer — no external deps. Handles the subset that
 * shows up in Claude output and library prompts/skills: headings, fenced code,
 * lists, blockquotes, rules, bold/italic/inline-code/links.
 *
 * Extracted from ClaudePane; also powers the Library editor's preview.
 */
import React from 'react';
import { claudeColors as colors } from './claude-shared';

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
function mdCachePut(key: string, value: React.ReactNode[]): React.ReactNode[] {
  if (mdCache.size >= MD_CACHE_MAX) {
    // Evict oldest entry (Map iteration order = insertion order)
    mdCache.delete(mdCache.keys().next().value!);
  }
  mdCache.set(key, value);
  return value;
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
      blocks.push(
        <div key={key++} style={{ margin: '6px 0' }}>
          {lang && (
            <div style={{
              fontSize: '0.6rem',
              color: colors.muted,
              backgroundColor: 'rgba(255,255,255,0.04)',
              padding: '2px 10px',
              borderRadius: '6px 6px 0 0',
              borderBottom: `1px solid ${colors.border}`,
              fontFamily: 'var(--claude-mono-font, monospace)',
            }}>
              {lang}
            </div>
          )}
          <pre style={{
            margin: 0,
            padding: '10px 12px',
            backgroundColor: 'rgba(0, 0, 0, 0.3)',
            borderRadius: lang ? '0 0 6px 6px' : '6px',
            fontSize: '0.75rem',
            lineHeight: 1.5,
            color: 'rgb(190, 200, 220)',
            fontFamily: 'var(--claude-mono-font, monospace)',
            overflowX: 'auto',
            whiteSpace: 'pre',
            border: `1px solid ${colors.border}`,
            borderTop: lang ? 'none' : undefined,
          }}>
            {codeLines.join('\n')}
          </pre>
        </div>
      );
      continue;
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
      !/^[-*_]{3,}\s*$/.test(lines[i].trim())
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
