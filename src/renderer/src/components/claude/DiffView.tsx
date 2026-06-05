import React from 'react';
import { claudeColors as colors } from '../claude-shared';

/** Render a unified diff view with both old and new lines */
export const DiffView: React.FC<{ oldStr: string; newStr: string; filePath?: string }> = ({ oldStr, newStr, filePath }) => {
  const fileName = filePath?.split(/[/\\]/).pop() ?? '';
  const oldLines = oldStr ? oldStr.split('\n') : [];
  const newLines = newStr ? newStr.split('\n') : [];
  const addedCount = newLines.length;
  const removedCount = oldLines.length;

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
            <span style={{ padding: '1px 8px 1px 0', whiteSpace: 'pre-wrap', wordBreak: 'break-all', flex: 1 }}>{line}</span>
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
            <span style={{ padding: '1px 8px 1px 0', whiteSpace: 'pre-wrap', wordBreak: 'break-all', flex: 1 }}>{line}</span>
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
