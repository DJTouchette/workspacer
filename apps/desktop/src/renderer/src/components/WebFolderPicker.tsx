import React, { useEffect, useState, useCallback } from 'react';
import { Folder, ArrowUp, Home, Check, X } from 'lucide-react';

interface Listing {
  path: string;
  parent: string;
  home: string;
  dirs: string[];
}

interface PendingPick {
  resolve: (path: string | null) => void;
  defaultPath?: string;
}

/**
 * Host filesystem browser for the web build. The desktop opens a native OS
 * dialog; in a browser that's impossible, so `webBackend.pickFolder` dispatches
 * a `web:pick-folder` event and this modal resolves it. It navigates the host's
 * directories via the `fs.listDir` capability (window.electronAPI.fsListDir).
 *
 * Mounted unconditionally in App — it's inert on desktop, since the native
 * pickFolder never fires the event.
 */
const WebFolderPicker: React.FC = () => {
  const [pending, setPending] = useState<PendingPick | null>(null);
  const [listing, setListing] = useState<Listing | null>(null);
  const [error, setError] = useState<string | null>(null);

  const browse = useCallback((path?: string) => {
    setError(null);
    window.electronAPI
      .fsListDir?.(path)
      .then((l) => setListing(l))
      .catch((e) => setError(e?.message || 'cannot read folder'));
  }, []);

  // Open on event.
  useEffect(() => {
    const handler = (e: Event) => {
      const detail = (e as CustomEvent).detail as PendingPick;
      setPending(detail);
      setListing(null);
      browse(detail.defaultPath);
    };
    window.addEventListener('web:pick-folder', handler);
    return () => window.removeEventListener('web:pick-folder', handler);
  }, [browse]);

  // Esc cancels.
  useEffect(() => {
    if (!pending) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') finish(null);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pending]);

  const finish = (path: string | null) => {
    pending?.resolve(path);
    setPending(null);
    setListing(null);
    setError(null);
  };

  if (!pending) return null;

  return (
    <div
      onMouseDown={() => finish(null)}
      style={{
        position: 'fixed',
        inset: 0,
        zIndex: 21000,
        background: 'var(--wks-overlay, rgba(0,0,0,0.5))',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
      }}
    >
      <div
        onMouseDown={(e) => e.stopPropagation()}
        style={{
          width: 460,
          maxWidth: '94vw',
          maxHeight: '80vh',
          display: 'flex',
          flexDirection: 'column',
          background: 'var(--wks-glass-strong)',
          backdropFilter: 'blur(var(--wks-glass-blur)) saturate(170%)',
          WebkitBackdropFilter: 'blur(var(--wks-glass-blur)) saturate(170%)',
          border: '1px solid var(--wks-glass-border)',
          borderRadius: 'var(--wks-radius-lg)',
          padding: 16,
          boxShadow: '0 16px 48px var(--wks-glass-shadow)',
          fontFamily: 'inherit',
        }}
      >
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 10 }}>
          <Folder size={15} color="var(--wks-text-primary)" />
          <div style={{ fontSize: '0.85rem', fontWeight: 600, color: 'var(--wks-text-primary)' }}>
            Choose a working directory
          </div>
        </div>

        {/* Current path + nav */}
        <div style={{ display: 'flex', gap: 6, alignItems: 'center', marginBottom: 8 }}>
          <button title="Home" onClick={() => browse(listing?.home)} style={iconBtn}>
            <Home size={13} />
          </button>
          <button
            title="Up one level"
            onClick={() => listing && browse(listing.parent)}
            style={iconBtn}
          >
            <ArrowUp size={13} />
          </button>
          <div
            style={{
              flex: 1,
              minWidth: 0,
              fontSize: '0.72rem',
              fontFamily: 'var(--wks-font-mono, monospace)',
              color: 'var(--wks-text-tertiary)',
              background: 'var(--wks-bg-base)',
              border: '1px solid var(--wks-border-input)',
              borderRadius: 4,
              padding: '6px 8px',
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
              direction: 'rtl',
              textAlign: 'left',
            }}
          >
            {listing?.path || '…'}
          </div>
        </div>

        {/* Directory list */}
        <div
          style={{
            flex: 1,
            overflowY: 'auto',
            minHeight: 160,
            maxHeight: '46vh',
            border: '1px solid var(--wks-border-subtle)',
            borderRadius: 6,
            padding: 4,
          }}
        >
          {error && (
            <div style={{ padding: 10, fontSize: '0.72rem', color: 'var(--wks-danger, #e05555)' }}>
              {error}
            </div>
          )}
          {!error && listing && listing.dirs.length === 0 && (
            <div style={{ padding: 10, fontSize: '0.72rem', color: 'var(--wks-text-faint)' }}>
              No subfolders here.
            </div>
          )}
          {!error &&
            listing?.dirs.map((name) => (
              <button
                key={name}
                onClick={() => browse(joinPath(listing.path, name))}
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 8,
                  width: '100%',
                  textAlign: 'left',
                  padding: '6px 8px',
                  border: 'none',
                  borderRadius: 4,
                  cursor: 'pointer',
                  background: 'transparent',
                  color: 'var(--wks-text-secondary)',
                  fontSize: '0.76rem',
                  fontFamily: 'inherit',
                }}
                onMouseEnter={(e) => (e.currentTarget.style.background = 'var(--wks-bg-input)')}
                onMouseLeave={(e) => (e.currentTarget.style.background = 'transparent')}
              >
                <Folder size={13} color="var(--wks-accent, #4a9eff)" /> {name}
              </button>
            ))}
        </div>

        {/* Actions */}
        <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8, marginTop: 12 }}>
          <button onClick={() => finish(null)} style={secondaryBtn}>
            <X size={13} /> Cancel
          </button>
          <button
            onClick={() => finish(listing?.path || null)}
            disabled={!listing}
            style={primaryBtn(!listing)}
          >
            <Check size={13} /> Use this folder
          </button>
        </div>
      </div>
    </div>
  );
};

/** Host-side join — '/' works on the daemon's POSIX hosts; the path is echoed
 *  back by fs.listDir anyway so a trailing-slash quirk self-corrects. */
function joinPath(base: string, name: string): string {
  return base.endsWith('/') ? base + name : `${base}/${name}`;
}

const iconBtn: React.CSSProperties = {
  display: 'inline-flex',
  alignItems: 'center',
  justifyContent: 'center',
  cursor: 'pointer',
  flexShrink: 0,
  background: 'var(--wks-bg-input)',
  color: 'var(--wks-text-tertiary)',
  border: '1px solid var(--wks-border-input)',
  borderRadius: 4,
  padding: '6px 8px',
};
const secondaryBtn: React.CSSProperties = {
  display: 'inline-flex',
  alignItems: 'center',
  gap: 5,
  fontSize: '0.76rem',
  fontFamily: 'inherit',
  cursor: 'pointer',
  background: 'transparent',
  color: 'var(--wks-text-tertiary)',
  border: '1px solid var(--wks-border-input)',
  borderRadius: 4,
  padding: '6px 12px',
};
const primaryBtn = (disabled: boolean): React.CSSProperties => ({
  display: 'inline-flex',
  alignItems: 'center',
  gap: 5,
  fontSize: '0.76rem',
  fontFamily: 'inherit',
  cursor: disabled ? 'default' : 'pointer',
  background: disabled ? 'var(--wks-bg-input)' : 'var(--wks-accent)',
  color: disabled ? 'var(--wks-text-faint)' : 'var(--wks-text-on-accent, #fff)',
  border: '1px solid var(--wks-border-input)',
  borderRadius: 4,
  padding: '6px 12px',
});

export default WebFolderPicker;
