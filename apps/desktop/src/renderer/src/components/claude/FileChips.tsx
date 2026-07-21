import React from 'react';
import { X } from 'lucide-react';
import { claudeColors as colors } from '../claude-shared';
import type { AttachedFile } from './fileAttachment';

export const FileChips: React.FC<{ files: AttachedFile[]; onRemove: (idx: number) => void }> = ({
  files,
  onRemove,
}) => {
  if (files.length === 0) return null;
  return (
    <div style={{ display: 'flex', flexWrap: 'wrap', gap: 4, padding: '0 0 4px 0' }}>
      {files.map((f, i) => (
        <span
          key={f.path}
          style={{
            display: 'inline-flex',
            alignItems: 'center',
            gap: 4,
            fontSize: '0.68rem',
            padding: '2px 8px',
            borderRadius: 'var(--wks-radius-pill)',
            backgroundColor: 'rgba(255,255,255,0.06)',
            border: `1px solid ${colors.borderSubtle}`,
            color: colors.text,
            maxWidth: 220,
          }}
        >
          <span
            style={{
              color:
                f.label === 'Image'
                  ? colors.purple
                  : f.label === 'PDF'
                    ? colors.error
                    : colors.accent,
              fontWeight: 600,
            }}
          >
            {f.label === 'Image' ? '\u{1F5BC}' : f.label === 'PDF' ? '\u{1F4C4}' : '\u{1F4CE}'}
          </span>
          <span
            style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}
            title={f.path}
          >
            {f.name}
          </span>
          <span
            onClick={() => onRemove(i)}
            style={{
              cursor: 'pointer',
              color: colors.muted,
              display: 'inline-flex',
              alignItems: 'center',
              marginLeft: 2,
            }}
          >
            <X size={12} strokeWidth={2} />
          </span>
        </span>
      ))}
    </div>
  );
};
