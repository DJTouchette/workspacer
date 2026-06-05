import React, { useState } from 'react';
import type { FileChange } from '../../types/claudeSession';
import { claudeColors as colors } from '../claude-shared';

export const InlineFilesSection: React.FC<{ fileChanges: FileChange[] }> = ({ fileChanges }) => {
  const [expanded, setExpanded] = useState(false);
  if (fileChanges.length === 0) return null;

  return (
    <div style={{
      margin: '4px 0 8px 0',
      borderRadius: 8,
      border: `1px solid ${colors.borderSubtle}`,
      backgroundColor: 'rgba(255,255,255,0.02)',
      overflow: 'hidden',
    }}>
      <div
        onClick={() => setExpanded(!expanded)}
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 6,
          padding: '5px 10px',
          cursor: 'pointer',
          fontSize: '0.7rem',
          color: colors.muted,
          userSelect: 'none',
        }}
      >
        <span style={{
          display: 'inline-block',
          width: 10,
          fontSize: '0.55rem',
          transition: 'transform 0.15s',
          transform: expanded ? 'rotate(90deg)' : 'rotate(0deg)',
        }}>
          {'▶'}
        </span>
        <span>{fileChanges.length} file{fileChanges.length !== 1 ? 's' : ''} changed</span>
      </div>
      {expanded && (
        <div style={{ padding: '0 10px 6px 10px' }}>
          {fileChanges.slice(-20).map((fc, i) => {
            const filename = fc.path.split('/').pop() ?? fc.path;
            return (
              <div key={i} style={{ fontSize: '0.7rem', padding: '1px 0', display: 'flex', gap: 6, alignItems: 'center' }}>
                <span style={{ color: fc.toolName === 'Write' ? colors.success : colors.warning, fontWeight: 600, width: 10, textAlign: 'center' }}>
                  {fc.toolName === 'Write' ? '+' : '~'}
                </span>
                <span style={{ color: colors.text, fontFamily: 'monospace' }}>{filename}</span>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
};
