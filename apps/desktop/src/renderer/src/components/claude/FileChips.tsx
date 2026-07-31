import React, { useEffect, useState } from 'react';
import { X } from 'lucide-react';
import { claudeColors as colors } from '../claude-shared';
import type { AttachedFile } from './fileAttachment';
import { useImagePreviews } from './imagePreviews';

// Cache control lives with the cache now; re-exported so existing importers
// (fileChips tests) keep working.
export { __clearPreviewCache } from './imagePreviews';

/** Edge of a thumbnail tile, in px. */
const THUMB_SIZE = 56;

const RemoveButton: React.FC<{ name: string; onClick: () => void; overlay?: boolean }> = ({
  name,
  onClick,
  overlay,
}) => (
  <button
    type="button"
    aria-label={`Remove ${name}`}
    onClick={onClick}
    style={{
      display: 'inline-flex',
      alignItems: 'center',
      justifyContent: 'center',
      cursor: 'pointer',
      border: 'none',
      padding: 0,
      color: overlay ? 'var(--wks-text-primary)' : colors.muted,
      ...(overlay
        ? {
            position: 'absolute',
            top: 3,
            right: 3,
            width: 16,
            height: 16,
            borderRadius: '50%',
            background: 'var(--wks-overlay)',
            backdropFilter: 'blur(4px)',
          }
        : { background: 'transparent', marginLeft: 2 }),
    }}
  >
    <X size={overlay ? 11 : 12} strokeWidth={2} />
  </button>
);

export const FileChips: React.FC<{ files: AttachedFile[]; onRemove: (idx: number) => void }> = ({
  files,
  onRemove,
}) => {
  const { previews } = useImagePreviews(
    files.filter((f) => f.label === 'Image').map((f) => f.path),
  );
  if (files.length === 0) return null;
  return (
    <div
      style={{
        display: 'flex',
        flexWrap: 'wrap',
        alignItems: 'center',
        gap: 6,
        padding: '0 0 4px 0',
      }}
    >
      {files.map((f, i) => {
        const preview = f.label === 'Image' ? previews[f.path] : undefined;
        // An image we can show renders as a tile; everything else — including
        // an image whose preview failed or is still decoding — stays a pill.
        if (preview) {
          return (
            <span
              key={f.path}
              title={f.path}
              style={{
                position: 'relative',
                width: THUMB_SIZE,
                height: THUMB_SIZE,
                borderRadius: 'var(--wks-radius-md)',
                overflow: 'hidden',
                border: `1px solid ${colors.borderSubtle}`,
                background: 'var(--wks-bg-elevated)',
                flexShrink: 0,
              }}
            >
              <img
                src={preview}
                alt={f.name}
                style={{ width: '100%', height: '100%', objectFit: 'cover', display: 'block' }}
              />
              <RemoveButton name={f.name} onClick={() => onRemove(i)} overlay />
            </span>
          );
        }
        return (
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
            <RemoveButton name={f.name} onClick={() => onRemove(i)} />
          </span>
        );
      })}
    </div>
  );
};
