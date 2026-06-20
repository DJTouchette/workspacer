import React, { useEffect, useState } from 'react';
import { QRCodeSVG } from 'qrcode.react';
import { Smartphone, Copy, Check, Eye, EyeOff } from 'lucide-react';

interface RemoteInfo {
  enabled: boolean;
  token: string;
  /** Lightweight client URL (the /remote single page), token included. */
  remoteUrl: string;
  /** Full-app (real renderer) URL at /app, token included. Empty when the web
   *  build hasn't been produced (run `npm run build:renderer:web`). */
  appUrl: string;
  /** Bare bus URL (no token) for diagnostics. */
  busUrl: string;
}

/**
 * Surfaces the hub's remote-sharing connection details so you can drive agents
 * from a phone. When sharing is on it shows a scannable QR of the token-bearing
 * URL (the fast path — point the camera, tap, done) plus copy buttons for the
 * URL and token. When it's off it explains how to turn it on. The data comes
 * straight from `getRemoteShareInfo()` over IPC; nothing is sent anywhere.
 */
const RemoteShareDialog: React.FC<{ onClose: () => void }> = ({ onClose }) => {
  const [info, setInfo] = useState<RemoteInfo | null>(null);
  const [loading, setLoading] = useState(true);
  const [copied, setCopied] = useState<string | null>(null);
  const [showToken, setShowToken] = useState(false);

  useEffect(() => {
    window.electronAPI.getRemoteInfo?.()
      .then((i) => setInfo(i))
      .catch(() => setInfo(null))
      .finally(() => setLoading(false));
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, [onClose]);

  const copy = async (label: string, text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(label);
      setTimeout(() => setCopied((c) => (c === label ? null : c)), 1500);
    } catch { /* clipboard blocked — nothing we can do */ }
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
          width: 440, maxWidth: '92vw', maxHeight: '90vh', overflowY: 'auto',
          backgroundColor: 'var(--wks-glass-strong)',
          backdropFilter: 'blur(var(--wks-glass-blur)) saturate(170%)',
          WebkitBackdropFilter: 'blur(var(--wks-glass-blur)) saturate(170%)',
          border: '1px solid var(--wks-glass-border)',
          borderRadius: 'var(--wks-radius-lg)',
          padding: 20,
          boxShadow: '0 16px 48px var(--wks-glass-shadow), inset 0 0 0 1.5px var(--wks-glass-highlight)',
          fontFamily: 'inherit',
        }}
      >
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 4 }}>
          <Smartphone size={16} color="var(--wks-text-primary)" />
          <div style={{ fontSize: '0.9rem', fontWeight: 600, color: 'var(--wks-text-primary)' }}>
            Remote control
          </div>
        </div>
        <div style={{ fontSize: '0.7rem', color: 'var(--wks-text-muted)', lineHeight: 1.5, marginBottom: 16 }}>
          Drive your agents — approve prompts, answer questions, send messages — from a phone or
          another computer on the same Tailscale tailnet.
        </div>

        {loading && (
          <div style={{ fontSize: '0.75rem', color: 'var(--wks-text-faint)' }}>Loading…</div>
        )}

        {!loading && !info && (
          <div style={{ fontSize: '0.75rem', color: 'var(--wks-danger, #e05555)' }}>
            Couldn't reach the hub for remote info.
          </div>
        )}

        {!loading && info && !info.enabled && <DisabledState />}

        {!loading && info && info.enabled && (
          <EnabledState info={info} copied={copied} showToken={showToken} onCopy={copy} onToggleToken={() => setShowToken((s) => !s)} />
        )}

        <div style={{ display: 'flex', justifyContent: 'flex-end', marginTop: 20 }}>
          <button onClick={onClose} style={secondaryBtnStyle}>Close</button>
        </div>
      </div>
    </div>
  );
};

function DisabledState() {
  return (
    <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-tertiary)', lineHeight: 1.6 }}>
      <div style={{
        display: 'inline-flex', alignItems: 'center', gap: 6, marginBottom: 12,
        padding: '3px 10px', borderRadius: 999,
        background: 'var(--wks-bg-input)', border: '1px solid var(--wks-border-input)',
        color: 'var(--wks-text-muted)', fontSize: '0.65rem', fontWeight: 600,
      }}>
        <span style={{ width: 6, height: 6, borderRadius: '50%', background: 'var(--wks-text-faint, #666)' }} />
        Remote sharing is OFF
      </div>
      <div style={{ marginBottom: 10 }}>
        By default the hub binds to localhost only. To share it, launch Workspacer with remote
        sharing enabled, then reopen this panel:
      </div>
      <Code>WORKSPACER_REMOTE_SHARE=1</Code>
      <div style={{ margin: '10px 0 6px', color: 'var(--wks-text-muted)' }}>
        Optionally pin it to your tailnet IP so it never listens on the LAN:
      </div>
      <Code>WORKSPACER_REMOTE_ADDR=100.x.y.z:7895</Code>
      <div style={{ marginTop: 12, color: 'var(--wks-text-faint)', fontSize: '0.68rem' }}>
        A shared token is generated automatically and stored at
        {' '}<code style={inlineCode}>~/.config/workspacer/remote-token</code>. Keep both devices on
        the same Tailscale network — the connection isn't encrypted on its own.
      </div>
    </div>
  );
}

function EnabledState({
  info, copied, showToken, onCopy, onToggleToken,
}: {
  info: RemoteInfo;
  copied: string | null;
  showToken: boolean;
  onCopy: (label: string, text: string) => void;
  onToggleToken: () => void;
}) {
  const hasApp = !!info.appUrl;
  // Prefer the full app when it's available; fall back to the lite client.
  const [mode, setMode] = useState<'app' | 'lite'>(hasApp ? 'app' : 'lite');
  const activeUrl = mode === 'app' && hasApp ? info.appUrl : info.remoteUrl;

  return (
    <div>
      <div style={{
        display: 'inline-flex', alignItems: 'center', gap: 6, marginBottom: 14,
        padding: '3px 10px', borderRadius: 999,
        background: 'var(--wks-bg-input)', border: '1px solid var(--wks-border-input)',
        color: 'var(--wks-success, #3fb950)', fontSize: '0.65rem', fontWeight: 600,
      }}>
        <span style={{
          width: 6, height: 6, borderRadius: '50%',
          background: 'var(--wks-success, #3fb950)', boxShadow: '0 0 5px var(--wks-success, #3fb950)',
        }} />
        Remote sharing is ON
      </div>

      {/* Full app vs lite client. The full app is the real renderer served at
          /app; lite is the single-page /remote client. */}
      {hasApp ? (
        <div style={{ display: 'flex', gap: 4, marginBottom: 14 }}>
          <ModeTab active={mode === 'app'} onClick={() => setMode('app')} title="The full Workspacer UI, in the browser">Full app</ModeTab>
          <ModeTab active={mode === 'lite'} onClick={() => setMode('lite')} title="Lightweight single-page control client">Lite</ModeTab>
        </div>
      ) : (
        <div style={{ fontSize: '0.66rem', color: 'var(--wks-text-faint)', marginBottom: 12, lineHeight: 1.5 }}>
          Showing the lite client. Build the full app with{' '}
          <code style={inlineCode}>npm run build:renderer:web</code> and restart to share the full UI.
        </div>
      )}

      {/* QR — the fast path. White quiet-zone box so it scans on any theme. */}
      <div style={{ display: 'flex', justifyContent: 'center', marginBottom: 14 }}>
        <div style={{ background: '#fff', padding: 12, borderRadius: 8, lineHeight: 0 }}>
          <QRCodeSVG value={activeUrl} size={188} level="M" marginSize={0} bgColor="#ffffff" fgColor="#000000" />
        </div>
      </div>
      <div style={{ textAlign: 'center', fontSize: '0.68rem', color: 'var(--wks-text-muted)', marginBottom: 16 }}>
        Scan with your phone's camera to open {mode === 'app' && hasApp ? 'the full app' : 'the control panel'}.
      </div>

      <CopyRow label="Connection URL" value={activeUrl} display={activeUrl}
        copied={copied === 'url'} onCopy={() => onCopy('url', activeUrl)} />

      <CopyRow
        label="Token"
        value={info.token}
        display={showToken ? info.token : '•'.repeat(Math.min(24, info.token.length || 8))}
        copied={copied === 'token'}
        onCopy={() => onCopy('token', info.token)}
        extra={
          <button onClick={onToggleToken} title={showToken ? 'Hide token' : 'Show token'} style={iconBtnStyle}>
            {showToken ? <EyeOff size={13} /> : <Eye size={13} />}
          </button>
        }
      />

      <div style={{
        marginTop: 14, paddingTop: 12, borderTop: '1px solid var(--wks-border-subtle)',
        fontSize: '0.68rem', color: 'var(--wks-text-faint)', lineHeight: 1.6,
      }}>
        The URL already includes the token, so anyone who can open it gets full control. Only share
        it over your trusted tailnet.
      </div>
    </div>
  );
}

function CopyRow({
  label, display, copied, onCopy, extra,
}: {
  label: string;
  value: string;
  display: string;
  copied: boolean;
  onCopy: () => void;
  extra?: React.ReactNode;
}) {
  return (
    <div style={{ marginBottom: 10 }}>
      <div style={{ fontSize: '0.62rem', color: 'var(--wks-text-muted)', marginBottom: 4 }}>{label}</div>
      <div style={{ display: 'flex', gap: 6, alignItems: 'stretch' }}>
        <div style={{
          flex: 1, minWidth: 0,
          fontSize: '0.72rem', fontFamily: 'var(--wks-font-mono, monospace)',
          color: 'var(--wks-text-tertiary)',
          background: 'var(--wks-bg-base)', border: '1px solid var(--wks-border-input)',
          borderRadius: 4, padding: '6px 8px',
          overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
          display: 'flex', alignItems: 'center',
        }}>
          {display}
        </div>
        {extra}
        <button onClick={onCopy} title={`Copy ${label.toLowerCase()}`} style={iconBtnStyle}>
          {copied ? <Check size={13} color="var(--wks-success, #3fb950)" /> : <Copy size={13} />}
        </button>
      </div>
    </div>
  );
}

function ModeTab({ active, onClick, title, children }: { active: boolean; onClick: () => void; title?: string; children: React.ReactNode }) {
  return (
    <button
      onClick={onClick}
      title={title}
      style={{
        flex: 1, cursor: 'pointer', fontSize: '0.72rem', fontWeight: 600, fontFamily: 'inherit',
        padding: '6px 10px', borderRadius: 6,
        background: active ? 'var(--wks-accent-soft, var(--wks-bg-input))' : 'transparent',
        color: active ? 'var(--wks-text-primary)' : 'var(--wks-text-muted)',
        border: `1px solid ${active ? 'var(--wks-accent, #4a9eff)' : 'var(--wks-border-input)'}`,
      }}
    >
      {children}
    </button>
  );
}

function Code({ children }: { children: React.ReactNode }) {
  return (
    <div style={{
      fontFamily: 'var(--wks-font-mono, monospace)', fontSize: '0.72rem',
      color: 'var(--wks-text-primary)',
      background: 'var(--wks-bg-base)', border: '1px solid var(--wks-border-input)',
      borderRadius: 4, padding: '6px 8px', overflowX: 'auto', whiteSpace: 'nowrap',
    }}>
      {children}
    </div>
  );
}

const inlineCode: React.CSSProperties = {
  fontFamily: 'var(--wks-font-mono, monospace)',
  background: 'var(--wks-bg-base)', padding: '1px 4px', borderRadius: 3,
};

const iconBtnStyle: React.CSSProperties = {
  display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
  cursor: 'pointer', flexShrink: 0,
  background: 'var(--wks-bg-input)', color: 'var(--wks-text-tertiary)',
  border: '1px solid var(--wks-border-input)', borderRadius: 4, padding: '0 9px',
};

const secondaryBtnStyle: React.CSSProperties = {
  fontSize: '0.78rem', fontFamily: 'inherit', cursor: 'pointer',
  background: 'transparent', color: 'var(--wks-text-tertiary)',
  border: '1px solid var(--wks-border-input)', borderRadius: 4, padding: '6px 14px',
};

export default RemoteShareDialog;
