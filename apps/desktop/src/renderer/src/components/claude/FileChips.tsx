import React, { useEffect, useState } from 'react';
import { X } from 'lucide-react';
import { claudeColors as colors } from '../claude-shared';
import type { AttachedFile } from './fileAttachment';

/** Edge of a thumbnail tile, in px. */
const THUMB_SIZE = 56;

/**
 * Path → thumbnail data URL, shared across every composer and kept across
 * mounts: attaching the same screenshot twice, or switching panes and back,
 * shouldn't re-decode it. `null` records a path we already failed on so a
 * missing/undecodable file isn't retried on every render.
 */
const previewCache = new Map<string, string | null>();
const MAX_CACHED_PREVIEWS = 64;

function rememberPreview(path: string, dataUrl: string | null): void {
  // Oldest-first eviction — Map preserves insertion order.
  if (previewCache.size >= MAX_CACHED_PREVIEWS) {
    const oldest = previewCache.keys().next().value;
    if (oldest !== undefined) previewCache.delete(oldest);
  }
  previewCache.set(path, dataUrl);
}

/** Only for tests, which need each case to start from a cold cache. */
export function __clearPreviewCache(): void {
  previewCache.clear();
}

function cachedPreviews(paths: string[]): Record<string, string> {
  const out: Record<string, string> = {};
  for (const p of paths) {
    const hit = previewCache.get(p);
    if (hit) out[p] = hit;
  }
  return out;
}

/**
 * Resolve thumbnails for the image attachments. Main does the decoding and
 * downscaling (see services/imagePreview.ts) because the renderer can't read
 * host paths directly — in dev it's served over http, where `file://` is
 * blocked.
 */
function useImagePreviews(files: AttachedFile[]): Record<string, string> {
  const imagePaths = files.filter((f) => f.label === 'Image').map((f) => f.path);
  // Effect key: the identity of the image set, not the array reference (which
  // changes on every parent render). NUL-separated because a filename may
  // legally contain a newline, which would split one path into two.
  const key = imagePaths.join('\u0000');
  const [previews, setPreviews] = useState<Record<string, string>>(() =>
    cachedPreviews(imagePaths),
  );

  useEffect(() => {
    const paths = key ? key.split('\u0000') : [];
    let cancelled = false;
    // Paint whatever is already cached immediately; the rest fill in below.
    setPreviews(cachedPreviews(paths));

    const missing = paths.filter((p) => !previewCache.has(p));
    if (missing.length === 0) return;

    const read = window.electronAPI?.readImagePreview;
    if (!read) {
      // No backend for previews (older web polyfill): chips stay iconic.
      missing.forEach((p) => rememberPreview(p, null));
      return;
    }
    void Promise.all(
      missing.map(async (p) => {
        try {
          const { dataUrl } = await read(p);
          rememberPreview(p, dataUrl);
        } catch {
          rememberPreview(p, null); // missing, too big, or undecodable
        }
      }),
    ).then(() => {
      if (!cancelled) setPreviews(cachedPreviews(paths));
    });

    return () => {
      cancelled = true;
    };
  }, [key]);

  return previews;
}

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
  const previews = useImagePreviews(files);
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
