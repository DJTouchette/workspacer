import React, { useEffect, useState } from 'react';
import { pluginRequirement, type PluginManifest } from '../types/plugin';
import { AlertTriangle } from './icons';

interface ExamplesGalleryDialogProps {
  /** Ids of already-installed plugins, so added examples show as "Added". */
  installedIds: string[];
  onClose: () => void;
  /** Called after a successful add so the host can refresh the plugin list. */
  onAdded?: (pluginId: string) => void;
}

function kindOf(m: PluginManifest): string {
  if (m.server) return 'sidecar';
  if (m.ui) return 'webview';
  return 'plugin';
}

/**
 * Browse and add the example plugins bundled with the app. Unlike the GitHub
 * installer, nothing is downloaded — the hub copies the example from inside the
 * app into the writable plugins dir and supervises it. Each card shows its
 * runtime requirement so the Python/Go sidecars aren't added blindly.
 */
const ExamplesGalleryDialog: React.FC<ExamplesGalleryDialogProps> = ({ installedIds, onClose, onAdded }) => {
  const [examples, setExamples] = useState<PluginManifest[] | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [added, setAdded] = useState<Set<string>>(() => new Set(installedIds));
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const handler = (e: KeyboardEvent) => { if (e.key === 'Escape') { e.preventDefault(); onClose(); } };
    window.addEventListener('keydown', handler, true);
    return () => window.removeEventListener('keydown', handler, true);
  }, [onClose]);

  useEffect(() => {
    let live = true;
    window.electronAPI.listExamplePlugins?.()
      .then((list) => { if (live) setExamples((list as PluginManifest[]) ?? []); })
      .catch(() => { if (live) setExamples([]); });
    return () => { live = false; };
  }, []);

  const add = async (id: string) => {
    setBusyId(id);
    setError(null);
    try {
      const res = await window.electronAPI.installExamplePlugin?.(id);
      if (res?.ok) {
        setAdded((prev) => new Set(prev).add(id));
        onAdded?.(res.plugin?.id ?? id);
      } else {
        setError(res?.error || 'Add failed');
      }
    } catch (e) {
      setError(String((e as Error)?.message ?? e));
    } finally {
      setBusyId(null);
    }
  };

  return (
    <div
      onMouseDown={onClose}
      style={{
        position: 'fixed', inset: 0, zIndex: 20000,
        backgroundColor: 'var(--wks-overlay, rgba(0,0,0,0.5))',
        display: 'flex', alignItems: 'center', justifyContent: 'center',
      }}
    >
      <div
        onMouseDown={(e) => e.stopPropagation()}
        style={{
          width: 560, maxWidth: '94vw', maxHeight: '82vh', display: 'flex', flexDirection: 'column',
          backgroundColor: 'var(--wks-glass-strong)',
          backdropFilter: 'blur(var(--wks-glass-blur)) saturate(170%)',
          WebkitBackdropFilter: 'blur(var(--wks-glass-blur)) saturate(170%)',
          border: '1px solid var(--wks-glass-border)',
          borderRadius: 'var(--wks-radius-lg)', padding: 20,
          boxShadow: '0 16px 48px var(--wks-glass-shadow), inset 0 0 0 1.5px var(--wks-glass-highlight)', fontFamily: 'inherit',
        }}
      >
        <div style={{ fontSize: '0.9rem', fontWeight: 600, color: 'var(--wks-text-primary)', marginBottom: 4 }}>
          Example plugins
        </div>
        <div style={{ fontSize: '0.68rem', color: 'var(--wks-text-muted)', marginBottom: 14 }}>
          Bundled with the app — adding one copies it locally, no download. Sidecar examples need the runtime noted on each.
        </div>

        {error && (
          <div style={{ marginBottom: 10, fontSize: '0.68rem', color: 'var(--wks-danger, #e05555)', wordBreak: 'break-word' }}>
            {error}
          </div>
        )}

        <div style={{ overflow: 'auto', display: 'flex', flexDirection: 'column', gap: 8, paddingRight: 2 }}>
          {examples === null && (
            <div style={{ padding: 20, textAlign: 'center', color: 'var(--wks-text-faint)', fontSize: '0.75rem' }}>Loading…</div>
          )}
          {examples?.length === 0 && (
            <div style={{ padding: 20, textAlign: 'center', color: 'var(--wks-text-faint)', fontSize: '0.75rem' }}>
              No bundled examples found.
            </div>
          )}
          {examples?.map((m) => {
            const req = pluginRequirement(m);
            const isAdded = added.has(m.id);
            const busy = busyId === m.id;
            return (
              <div key={m.id} style={{
                border: '1px solid var(--wks-border-subtle)', borderRadius: 8, padding: '10px 12px',
                background: 'var(--wks-bg-surface)', display: 'flex', alignItems: 'center', gap: 10,
              }}>
                <div style={{ minWidth: 0, flex: 1 }}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                    <span style={{ fontSize: '0.82rem', fontWeight: 600 }}>{m.name || m.id}</span>
                    <span style={{ fontSize: '0.6rem', color: 'var(--wks-text-faint)', border: '1px solid var(--wks-border-subtle)', borderRadius: 3, padding: '1px 5px' }}>{kindOf(m)}</span>
                  </div>
                  <div style={{ fontSize: '0.62rem', color: 'var(--wks-text-faint)', marginTop: 2 }}>{m.id}</div>
                  <div style={{
                    marginTop: 4, fontSize: '0.62rem',
                    color: req.warn ? 'var(--wks-warning, #e0a000)' : 'var(--wks-text-muted)',
                    display: 'inline-flex', alignItems: 'center', gap: 4,
                  }}>
                    {req.warn && <AlertTriangle size={11} strokeWidth={2} />}{req.label}
                  </div>
                </div>
                <button
                  onClick={() => !isAdded && add(m.id)}
                  disabled={isAdded || busy}
                  style={{
                    fontSize: '0.72rem', fontFamily: 'inherit', flexShrink: 0,
                    cursor: (isAdded || busy) ? 'default' : 'pointer',
                    background: isAdded ? 'transparent' : 'var(--wks-accent)',
                    color: isAdded ? 'var(--wks-text-faint)' : 'var(--wks-text-on-accent, #fff)',
                    border: isAdded ? '1px solid var(--wks-border-input)' : 'none',
                    borderRadius: 5, padding: '5px 14px', fontWeight: 600,
                  }}
                >{isAdded ? 'Added' : busy ? 'Adding…' : 'Add'}</button>
              </div>
            );
          })}
        </div>

        <div style={{ display: 'flex', justifyContent: 'flex-end', marginTop: 16 }}>
          <button
            onClick={onClose}
            style={{
              fontSize: '0.78rem', fontFamily: 'inherit', cursor: 'pointer',
              background: 'transparent', color: 'var(--wks-text-tertiary)',
              border: '1px solid var(--wks-border-input)', borderRadius: 4, padding: '6px 14px',
            }}
          >Done</button>
        </div>
      </div>
    </div>
  );
};

export default ExamplesGalleryDialog;
