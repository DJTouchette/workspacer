import React, { useState, useEffect } from 'react';
import { pluginRequirement, type PluginManifest } from '../types/plugin';
import { AlertTriangle } from './icons';
import { PluginPermissions } from './plugin/PluginPermissions';

interface PluginInstallDialogProps {
  onClose: () => void;
  /** Called after a successful install so the host can refresh / surface it. */
  onInstalled?: (pluginId: string) => void;
}

/**
 * Install a plugin from a GitHub URL, in two steps so the user isn't running
 * unknown code blind:
 *   1. Inspect — the hub downloads the repo and reads its plugin.json (no code
 *      runs). We show what the plugin contributes and what it requires
 *      (Go/Rust/Python/Node), so a build that needs a missing toolchain — or a
 *      sidecar that needs a missing runtime — is flagged before committing.
 *   2. Install — download again, run any build step, supervise the sidecar.
 */
const PluginInstallDialog: React.FC<PluginInstallDialogProps> = ({ onClose, onInstalled }) => {
  const [url, setUrl] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Live install stage (downloading / extracting / building) from the hub bus.
  const [stage, setStage] = useState<string | null>(null);
  // The inspected manifest — once set, we show the preview/confirm step.
  const [preview, setPreview] = useState<PluginManifest | null>(null);

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener('keydown', handler, true);
    return () => window.removeEventListener('keydown', handler, true);
  }, [onClose]);

  // Reflect the hub's install progress so a long build step isn't a frozen button.
  useEffect(() => {
    const off = window.electronAPI.onHubEvent?.((ev) => {
      if (ev.type !== 'plugin.install.progress') return;
      const d = ev.data as { stage?: string } | undefined;
      if (d?.stage) setStage(d.stage);
    });
    return () => off?.();
  }, []);

  // Step 1: download + read the manifest without installing.
  const inspect = async () => {
    const trimmed = url.trim();
    if (!trimmed || busy) return;
    setBusy(true);
    setError(null);
    try {
      const res = await window.electronAPI.inspectPlugin?.(trimmed);
      if (res?.ok && res.plugin) {
        setPreview(res.plugin);
      } else {
        setError(res?.error || 'Could not read the plugin');
      }
    } catch (e) {
      setError(String((e as Error)?.message ?? e));
    } finally {
      setBusy(false);
    }
  };

  // Step 2: actually install (download → build → supervise).
  const install = async () => {
    const trimmed = url.trim();
    if (!trimmed || busy) return;
    setBusy(true);
    setError(null);
    setStage(null);
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

  const req = preview ? pluginRequirement(preview) : null;

  return (
    <div
      onMouseDown={onClose}
      style={{
        position: 'fixed',
        inset: 0,
        zIndex: 20000,
        backgroundColor: 'var(--wks-overlay, rgba(0,0,0,0.5))',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
      }}
    >
      <div
        onMouseDown={(e) => e.stopPropagation()}
        style={{
          width: 460,
          maxWidth: '92vw',
          backgroundColor: 'var(--wks-glass-strong)',
          backdropFilter: 'blur(var(--wks-glass-blur)) saturate(170%)',
          WebkitBackdropFilter: 'blur(var(--wks-glass-blur)) saturate(170%)',
          border: '1px solid var(--wks-glass-border)',
          borderRadius: 'var(--wks-radius-lg)',
          padding: 20,
          boxShadow:
            '0 16px 48px var(--wks-glass-shadow), inset 0 0 0 1.5px var(--wks-glass-highlight)',
          fontFamily: 'inherit',
        }}
      >
        <div
          style={{
            fontSize: '0.9rem',
            fontWeight: 600,
            color: 'var(--wks-text-primary)',
            marginBottom: 4,
          }}
        >
          Install plugin from GitHub
        </div>

        {!preview ? (
          <>
            <div style={{ fontSize: '0.68rem', color: 'var(--wks-text-muted)', marginBottom: 14 }}>
              Paste a repo URL or <code>owner/repo</code>. It must contain a{' '}
              <code>plugin.json</code>.
            </div>
            <input
              autoFocus
              value={url}
              onChange={(e) => setUrl(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') inspect();
                if (e.key === 'Escape') onClose();
              }}
              placeholder="https://github.com/owner/repo  or  owner/repo"
              style={{
                width: '100%',
                boxSizing: 'border-box',
                fontSize: '0.8rem',
                fontFamily: 'inherit',
                background: 'var(--wks-bg-base)',
                color: 'var(--wks-text-primary)',
                border: '1px solid var(--wks-border-input)',
                borderRadius: 4,
                padding: '7px 9px',
              }}
            />
            <div style={{ marginTop: 12, fontSize: '0.62rem', color: 'var(--wks-text-faint)' }}>
              We'll read its manifest first and show what it does before anything runs.
            </div>
          </>
        ) : (
          <>
            <div style={{ fontSize: '0.68rem', color: 'var(--wks-text-muted)', marginBottom: 12 }}>
              Review before installing — this will run the plugin's code on your machine.
            </div>
            <div
              style={{
                border: '1px solid var(--wks-border-subtle)',
                borderRadius: 'var(--wks-radius-md)',
                padding: 12,
                background: 'var(--wks-bg-surface)',
              }}
            >
              <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                <span style={{ fontSize: '0.85rem', fontWeight: 600 }}>
                  {preview.name || preview.id}
                </span>
                <span
                  style={{
                    fontSize: '0.6rem',
                    color: 'var(--wks-text-faint)',
                    border: '1px solid var(--wks-border-subtle)',
                    borderRadius: 3,
                    padding: '1px 5px',
                  }}
                >
                  {preview.server ? 'sidecar' : preview.ui ? 'webview' : 'plugin'}
                </span>
              </div>
              <div style={{ fontSize: '0.62rem', color: 'var(--wks-text-faint)', marginTop: 2 }}>
                {preview.id}
              </div>
              <div
                style={{
                  marginTop: 8,
                  display: 'flex',
                  gap: 14,
                  flexWrap: 'wrap',
                  fontSize: '0.62rem',
                  color: 'var(--wks-text-muted)',
                }}
              >
                <span>
                  {preview.panes?.length ?? 0} pane{(preview.panes?.length ?? 0) === 1 ? '' : 's'}
                  {preview.panes?.length ? ': ' + preview.panes.map((x) => x.title).join(', ') : ''}
                </span>
              </div>

              {/* Itemized permissions — what the plugin can do on the bus, so
                  the install click is informed consent (the hub enforces
                  exactly these). */}
              <div
                style={{
                  marginTop: 12,
                  paddingTop: 10,
                  borderTop: '1px solid var(--wks-border-subtle)',
                }}
              >
                <div
                  style={{
                    fontSize: '0.6rem',
                    fontWeight: 700,
                    textTransform: 'uppercase',
                    letterSpacing: '0.05em',
                    color: 'var(--wks-text-muted)',
                    marginBottom: 8,
                  }}
                >
                  Permissions it's requesting
                </div>
                <PluginPermissions manifest={preview} />
              </div>

              {req && (
                <div
                  style={{
                    marginTop: 8,
                    fontSize: '0.64rem',
                    fontWeight: 600,
                    color: req.warn ? 'var(--wks-warning, #e0a000)' : 'var(--wks-success, #3fb950)',
                    display: 'inline-flex',
                    alignItems: 'center',
                    gap: 5,
                  }}
                >
                  {req.warn && <AlertTriangle size={12} strokeWidth={2} />}
                  {req.label}
                  {req.warn && (
                    <span style={{ fontWeight: 400, color: 'var(--wks-text-muted)' }}>
                      {' '}
                      — must be installed on this machine
                    </span>
                  )}
                </div>
              )}
            </div>
          </>
        )}

        <div
          style={{
            marginTop: 12,
            padding: '8px 10px',
            borderRadius: 'var(--wks-radius-sm)',
            background: 'var(--wks-bg-input)',
            border: '1px solid var(--wks-border-subtle)',
            fontSize: '0.65rem',
            lineHeight: 1.5,
            color: 'var(--wks-text-muted)',
          }}
        >
          <span
            style={{
              display: 'inline-flex',
              alignItems: 'center',
              gap: 4,
              verticalAlign: 'text-bottom',
              color: 'var(--wks-danger, #e05555)',
              fontWeight: 600,
            }}
          >
            <AlertTriangle size={12} strokeWidth={2} /> Runs code from the internet.
          </span>{' '}
          Installing starts the plugin's process on your machine — like a VS Code extension. Only
          install plugins you trust.
        </div>

        {error && (
          <div
            style={{
              marginTop: 10,
              fontSize: '0.68rem',
              color: 'var(--wks-danger, #e05555)',
              wordBreak: 'break-word',
            }}
          >
            {error}
          </div>
        )}

        <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8, marginTop: 18 }}>
          <button
            onClick={
              preview
                ? () => {
                    setPreview(null);
                    setError(null);
                  }
                : onClose
            }
            disabled={busy}
            style={{
              fontSize: '0.78rem',
              fontFamily: 'inherit',
              cursor: busy ? 'default' : 'pointer',
              background: 'transparent',
              color: 'var(--wks-text-tertiary)',
              border: '1px solid var(--wks-border-input)',
              borderRadius: 4,
              padding: '6px 14px',
            }}
          >
            {preview ? 'Back' : 'Cancel'}
          </button>
          <button
            onClick={preview ? install : inspect}
            disabled={!url.trim() || busy}
            style={{
              fontSize: '0.78rem',
              fontFamily: 'inherit',
              cursor: !url.trim() || busy ? 'default' : 'pointer',
              background: !url.trim() || busy ? 'var(--wks-bg-input)' : 'var(--wks-accent)',
              color:
                !url.trim() || busy ? 'var(--wks-text-faint)' : 'var(--wks-text-on-accent, #fff)',
              border: 'none',
              borderRadius: 4,
              padding: '6px 14px',
              fontWeight: 600,
            }}
          >
            {busy
              ? preview
                ? `${stage ? stage[0].toUpperCase() + stage.slice(1) : 'Installing'}…`
                : 'Reading…'
              : preview
                ? 'Install'
                : 'Continue'}
          </button>
        </div>
      </div>
    </div>
  );
};

export default PluginInstallDialog;
