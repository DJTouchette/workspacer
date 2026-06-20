import React, { useState, useEffect } from 'react';
import { AlertTriangle } from './icons';

interface PluginInstallDialogProps {
  onClose: () => void;
  /** Called after a successful install so the host can refresh / surface it. */
  onInstalled?: (pluginId: string) => void;
}

/**
 * Install a plugin from a GitHub URL. The hub downloads the repo tarball,
 * extracts it, runs any declared build step, and supervises its sidecar — i.e.
 * it RUNS code from the internet, so this dialog makes that explicit.
 */
const PluginInstallDialog: React.FC<PluginInstallDialogProps> = ({ onClose, onInstalled }) => {
  const [url, setUrl] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const handler = (e: KeyboardEvent) => { if (e.key === 'Escape') { e.preventDefault(); onClose(); } };
    window.addEventListener('keydown', handler, true);
    return () => window.removeEventListener('keydown', handler, true);
  }, [onClose]);

  const install = async () => {
    const trimmed = url.trim();
    if (!trimmed || busy) return;
    setBusy(true);
    setError(null);
    try {
      const res = await window.electronAPI.installPlugin?.(trimmed);
      if (res?.ok) {
        onInstalled?.(res.plugin?.id ?? '');
        onClose();
      } else {
        setError(res?.error || 'Install failed');
      }
    } catch (e) {
      setError(String((e as Error)?.message ?? e));
    } finally {
      setBusy(false);
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
          width: 460, maxWidth: '92vw',
          backgroundColor: 'var(--wks-glass-strong)',
          backdropFilter: 'blur(var(--wks-glass-blur)) saturate(170%)',
          WebkitBackdropFilter: 'blur(var(--wks-glass-blur)) saturate(170%)',
          border: '1px solid var(--wks-glass-border)',
          borderRadius: 'var(--wks-radius-lg)', padding: 20,
          boxShadow: '0 16px 48px var(--wks-glass-shadow), inset 0 0 0 1.5px var(--wks-glass-highlight)', fontFamily: 'inherit',
        }}
      >
        <div style={{ fontSize: '0.9rem', fontWeight: 600, color: 'var(--wks-text-primary)', marginBottom: 4 }}>
          Install plugin from GitHub
        </div>
        <div style={{ fontSize: '0.68rem', color: 'var(--wks-text-muted)', marginBottom: 14 }}>
          Paste a repo URL or <code>owner/repo</code>. It must contain a <code>plugin.json</code>.
        </div>

        <input
          autoFocus
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          onKeyDown={(e) => { if (e.key === 'Enter') install(); if (e.key === 'Escape') onClose(); }}
          placeholder="https://github.com/owner/repo  or  owner/repo"
          style={{
            width: '100%', boxSizing: 'border-box', fontSize: '0.8rem', fontFamily: 'inherit',
            background: 'var(--wks-bg-base)', color: 'var(--wks-text-primary)',
            border: '1px solid var(--wks-border-input)', borderRadius: 4, padding: '7px 9px',
          }}
        />

        <div style={{
          marginTop: 12, padding: '8px 10px', borderRadius: 5,
          background: 'var(--wks-bg-input)', border: '1px solid var(--wks-border-subtle)',
          fontSize: '0.65rem', lineHeight: 1.5, color: 'var(--wks-text-muted)',
        }}>
          <span style={{ display: 'inline-flex', alignItems: 'center', gap: 4, verticalAlign: 'text-bottom', color: 'var(--wks-danger, #e05555)', fontWeight: 600 }}><AlertTriangle size={12} strokeWidth={2} /> Runs code from the internet.</span>{' '}
          Installing starts the plugin's server process on your machine — like a VS Code extension.
          Only install plugins you trust.
        </div>

        {error && (
          <div style={{ marginTop: 10, fontSize: '0.68rem', color: 'var(--wks-danger, #e05555)', wordBreak: 'break-word' }}>
            {error}
          </div>
        )}

        <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8, marginTop: 18 }}>
          <button
            onClick={onClose}
            style={{
              fontSize: '0.78rem', fontFamily: 'inherit', cursor: 'pointer',
              background: 'transparent', color: 'var(--wks-text-tertiary)',
              border: '1px solid var(--wks-border-input)', borderRadius: 4, padding: '6px 14px',
            }}
          >Cancel</button>
          <button
            onClick={install}
            disabled={!url.trim() || busy}
            style={{
              fontSize: '0.78rem', fontFamily: 'inherit', cursor: (!url.trim() || busy) ? 'default' : 'pointer',
              background: (!url.trim() || busy) ? 'var(--wks-bg-input)' : 'var(--wks-accent)',
              color: (!url.trim() || busy) ? 'var(--wks-text-faint)' : 'var(--wks-text-on-accent, #fff)',
              border: 'none', borderRadius: 4, padding: '6px 14px', fontWeight: 600,
            }}
          >{busy ? 'Installing…' : 'Install'}</button>
        </div>
      </div>
    </div>
  );
};

export default PluginInstallDialog;
