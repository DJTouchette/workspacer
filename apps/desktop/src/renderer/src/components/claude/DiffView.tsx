import React from 'react';
import { claudeColors as colors } from '../claude-shared';
import { langForPath } from '../../lib/diff/highlight';
import { useHighlight, renderLine } from './highlight';

/** Render a unified diff view with both old and new lines */
export const DiffView: React.FC<{ oldStr: string; newStr: string; filePath?: string }> = ({ oldStr, newStr, filePath }) => {
  const fileName = filePath?.split(/[/\\]/).pop() ?? '';
  const oldLines = oldStr ? oldStr.split('\n') : [];
  const newLines = newStr ? newStr.split('\n') : [];
  const addedCount = newLines.length;
  const removedCount = oldLines.length;
  // Syntax tokens per side; backgrounds stay our add/del tints, foreground comes
  // from the grammar (falls back to the diff color until tokens load).
  const lang = langForPath(filePath ?? '');
  const oldTokens = useHighlight(oldStr, lang);
  const newTokens = useHighlight(newStr, lang);

  return (
    <div style={{
      margin: '6px 0',
      borderRadius: 6,
      overflow: 'hidden',
      border: `1px solid ${colors.borderSubtle}`,
      fontSize: '0.75rem',
      fontFamily: 'var(--claude-mono-font, monospace)',
    }}>
      {fileName && (
        <div style={{
          padding: '5px 12px',
          backgroundColor: 'rgba(255,255,255,0.03)',
          color: colors.text,
          fontSize: '0.72rem',
          fontWeight: 600,
          borderBottom: `1px solid ${colors.borderSubtle}`,
          display: 'flex',
          alignItems: 'center',
          gap: 8,
        }}>
          <span>{fileName}</span>
          {removedCount > 0 && <span style={{ color: colors.error, fontSize: '0.65rem', fontWeight: 400 }}>-{removedCount}</span>}
          {addedCount > 0 && <span style={{ color: colors.success, fontSize: '0.65rem', fontWeight: 400 }}>+{addedCount}</span>}
        </div>
      )}
      <div style={{ maxHeight: 600, overflow: 'auto' }}>
        {oldLines.map((line, i) => (
          <div key={`old-${i}`} style={{
            display: 'flex',
            backgroundColor: 'rgba(248, 113, 113, 0.08)',
            color: 'rgb(248, 150, 150)',
            lineHeight: 1.5,
          }}>
            <span style={{ color: 'rgba(248,150,150,0.4)', userSelect: 'none', width: 36, minWidth: 36, textAlign: 'right', padding: '1px 6px 1px 0', fontSize: '0.65rem', borderRight: '1px solid rgba(248,113,113,0.15)' }}>{i + 1}</span>
            <span style={{ color: colors.error, userSelect: 'none', width: 16, minWidth: 16, textAlign: 'center', padding: '1px 0' }}>-</span>
            <span style={{ padding: '1px 8px 1px 0', whiteSpace: 'pre-wrap', wordBreak: 'break-all', flex: 1 }}>{renderLine(oldTokens?.[i], line)}</span>
          </div>
        ))}
        {newLines.map((line, i) => (
          <div key={`new-${i}`} style={{
            display: 'flex',
            backgroundColor: 'rgba(74, 222, 128, 0.08)',
            color: 'rgb(150, 230, 170)',
            lineHeight: 1.5,
          }}>
            <span style={{ color: 'rgba(150,230,170,0.4)', userSelect: 'none', width: 36, minWidth: 36, textAlign: 'right', padding: '1px 6px 1px 0', fontSize: '0.65rem', borderRight: '1px solid rgba(74,222,128,0.15)' }}>{i + 1}</span>
            <span style={{ color: colors.success, userSelect: 'none', width: 16, minWidth: 16, textAlign: 'center', padding: '1px 0' }}>+</span>
            <span style={{ padding: '1px 8px 1px 0', whiteSpace: 'pre-wrap', wordBreak: 'break-all', flex: 1 }}>{renderLine(newTokens?.[i], line)}</span>
          </div>
        ))}
      </div>
    </div>
  );
};

/** Check if a tool call has diff-able content */
export function hasDiff(tc: { name: string; input?: Record<string, unknown> }): boolean {
  return (tc.name === 'Edit' || tc.name === 'MultiEdit') && !!(tc.input?.old_string || tc.input?.new_string);
}

/** Cap how many read lines we render inline; the rest is summarized. */
const READ_PREVIEW_MAX = 200;

/**
 * Read-only, line-numbered preview of the lines a Read tool call returned —
 * the read analogue of DiffView. The response is in `cat -n` format
 * (`␣␣␣123⇥content`); we parse the gutter number off each line and fall back
 * to a sequential index if a line doesn't match.
 */
export const ReadView: React.FC<{ response: string; filePath?: string }> = ({ response, filePath }) => {
  const fileName = filePath?.split(/[/\\]/).pop() ?? '';
  const lines = response.split('\n');
  if (lines.length && lines[lines.length - 1] === '') lines.pop(); // trailing newline
  const shown = lines.slice(0, READ_PREVIEW_MAX);
  const hidden = lines.length - shown.length;
  // Strip the `cat -n` gutter to a {num, text} pair, then tokenize the file
  // content as one document so highlighting has cross-line context.
  const parsed = shown.map((raw, i) => {
    const m = /^\s*(\d+)\t(.*)$/.exec(raw);
    return { num: m ? m[1] : String(i + 1), text: m ? m[2] : raw };
  });
  const lang = langForPath(filePath ?? '');
  const tokens = useHighlight(parsed.map((p) => p.text).join('\n'), lang);

  return (
    <div style={{
      margin: '6px 0',
      borderRadius: 6,
      overflow: 'hidden',
      border: `1px solid ${colors.borderSubtle}`,
      fontSize: '0.75rem',
      fontFamily: 'var(--claude-mono-font, monospace)',
    }}>
      {fileName && (
        <div style={{
          padding: '5px 12px',
          backgroundColor: 'rgba(255,255,255,0.03)',
          color: colors.text,
          fontSize: '0.72rem',
          fontWeight: 600,
          borderBottom: `1px solid ${colors.borderSubtle}`,
          display: 'flex',
          alignItems: 'center',
          gap: 8,
        }}>
          <span>{fileName}</span>
          <span style={{ color: colors.muted, fontSize: '0.65rem', fontWeight: 400 }}>{lines.length} lines</span>
        </div>
      )}
      <div style={{ maxHeight: 600, overflow: 'auto' }}>
        {parsed.map(({ num, text }, i) => (
          <div key={i} style={{ display: 'flex', lineHeight: 1.5 }}>
            <span style={{
              color: colors.mutedDim, userSelect: 'none', width: 44, minWidth: 44,
              textAlign: 'right', padding: '1px 8px 1px 0', fontSize: '0.65rem',
              borderRight: `1px solid ${colors.borderSubtle}`,
            }}>{num}</span>
            <span style={{ color: colors.text, padding: '1px 8px', whiteSpace: 'pre-wrap', wordBreak: 'break-all', flex: 1 }}>{renderLine(tokens?.[i], text)}</span>
          </div>
        ))}
        {hidden > 0 && (
          <div style={{ padding: '2px 8px 2px 52px', color: colors.muted, fontSize: '0.65rem' }}>
            +{hidden} more line{hidden !== 1 ? 's' : ''}
          </div>
        )}
      </div>
    </div>
  );
};

/** Check if a tool call has read content to preview. */
export function hasRead(tc: { name: string; response?: unknown }): boolean {
  return tc.name === 'Read' && typeof tc.response === 'string' && tc.response.length > 0;
}
