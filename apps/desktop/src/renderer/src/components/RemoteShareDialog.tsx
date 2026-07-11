import React, { useCallback, useEffect, useState } from 'react';
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
  /** Local daemons adopted from an external `workspacer serve` on this machine. */
  hubAdopted?: boolean;
  claudemonAdopted?: boolean;
  /** Configured "connect to remote server" target (client mode), or null. */
  remoteClient?: { httpUrl: string; busUrl: string; token: string } | null;
}

interface TailscaleInfoUI {
  available: boolean;
  magicName: string | null;
  serveActive: boolean;
  canServe: boolean;
  hint?: string;
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
  const [toggling, setToggling] = useState(false);

  // Flip remote sharing on/off. Main persists the choice and restarts the hub
  // (re-binding loopback ⇄ tailnet), then returns fresh share info.
  const toggleShare = async (enabled: boolean) => {
    setToggling(true);
    try {
      const next = await window.electronAPI.setRemoteShare?.(enabled);
      if (next) setInfo(next);
    } catch {
      /* leave current state; the badge still reflects reality on next open */
    } finally {
      setToggling(false);
    }
  };

  useEffect(() => {
    window.electronAPI
      .getRemoteInfo?.()
      .then((i) => setInfo(i))
      .catch(() => setInfo(null))
      .finally(() => setLoading(false));
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, [onClose]);

  const copy = async (label: string, text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(label);
      setTimeout(() => setCopied((c) => (c === label ? null : c)), 1500);
    } catch {
      /* clipboard blocked — nothing we can do */
    }
  };

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
          width: 440,
          maxWidth: '92vw',
          maxHeight: '90vh',
          overflowY: 'auto',
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
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 4 }}>
          <Smartphone size={16} color="var(--wks-text-primary)" />
          <div style={{ fontSize: '0.9rem', fontWeight: 600, color: 'var(--wks-text-primary)' }}>
            Remote control
          </div>
        </div>
        <div
          style={{
            fontSize: '0.7rem',
            color: 'var(--wks-text-muted)',
            lineHeight: 1.5,
            marginBottom: 16,
          }}
        >
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

        {/* Client mode: this app IS the remote — the local sharing controls are
            moot (no local hub is running), so show only the connection state. */}
        {!loading && info && !info.remoteClient && (
          <>
            {info.hubAdopted && <AdoptedNote claudemon={!!info.claudemonAdopted} />}
            {!info.enabled && <DisabledState busy={toggling} onStart={() => toggleShare(true)} />}
            {info.enabled && (
              <EnabledState
                info={info}
                copied={copied}
                showToken={showToken}
                busy={toggling}
                onCopy={copy}
                onToggleToken={() => setShowToken((s) => !s)}
                onStop={() => toggleShare(false)}
              />
            )}
          </>
        )}

        {!loading && info && <RemoteClientSection info={info} />}

        <div style={{ display: 'flex', justifyContent: 'flex-end', marginTop: 20 }}>
          <button onClick={onClose} style={secondaryBtnStyle}>
            Close
          </button>
        </div>
      </div>
    </div>
  );
};

/** Shown when the local hub/claudemon were ADOPTED from a `workspacer serve`
 *  running on this machine: the app is a guest of those daemons — it didn't
 *  spawn them and quitting won't stop them. Purely informational. */
function AdoptedNote({ claudemon }: { claudemon: boolean }) {
  return (
    <div
      style={{
        margin: '0 0 14px',
        padding: '8px 12px',
        borderRadius: 'var(--wks-radius-md)',
        background: 'var(--wks-bg-input)',
        border: '1px solid var(--wks-border-subtle, var(--wks-border-input))',
        fontSize: '0.66rem',
        color: 'var(--wks-text-tertiary)',
        lineHeight: 1.5,
      }}
    >
      Using the daemons of a <code style={inlineCode}>workspacer serve</code> already running on
      this machine (hub{claudemon ? ' + claudemon' : ''} adopted). Quitting the app leaves the
      server running.
    </div>
  );
}

/**
 * "Connect to remote server" — run this app as a client of an external
 * `workspacer serve` (typically over a Tailscale tailnet). Connecting persists
 * the target and relaunches: main skips the local daemons and the renderer
 * boots against the remote hub bus (see backend/install.ts). Disconnecting
 * clears the setting and relaunches back into local mode. Rendered only where
 * the setting can be persisted (the desktop preload exposes setRemoteServer).
 */
function RemoteClientSection({ info }: { info: RemoteInfo }) {
  const [url, setUrl] = useState('');
  const [token, setToken] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  if (!window.electronAPI.setRemoteServer) return null;

  const apply = async (setting: { url: string; token: string } | null) => {
    setBusy(true);
    setError(null);
    try {
      const r = await window.electronAPI.setRemoteServer!(setting);
      if (!r.ok) {
        setError(r.error || 'Could not save the server setting.');
        return;
      }
      // Applying needs a fresh boot (daemon-vs-remote is decided at startup).
      await window.electronAPI.appRelaunch?.();
    } catch {
      setError('Could not save the server setting.');
    } finally {
      setBusy(false);
    }
  };

  const connected = info.remoteClient;
  return (
    <div
      style={{
        marginTop: 18,
        paddingTop: 14,
        borderTop: '1px solid var(--wks-border-subtle)',
      }}
    >
      <div
        style={{
          fontSize: '0.78rem',
          fontWeight: 600,
          color: 'var(--wks-text-primary)',
          marginBottom: 4,
        }}
      >
        Connect to a remote server
      </div>
      {connected ? (
        <>
          <div
            style={{
              fontSize: '0.68rem',
              color: 'var(--wks-text-tertiary)',
              lineHeight: 1.6,
              marginBottom: 10,
            }}
          >
            This app is a client of <code style={inlineCode}>{connected.httpUrl}</code> — agents run
            there, and no local daemons were started. Disconnect to go back to running agents on
            this machine.
          </div>
          <button onClick={() => apply(null)} disabled={busy} style={dangerBtnStyle(busy)}>
            {busy ? 'Disconnecting…' : 'Disconnect (restarts the app)'}
          </button>
        </>
      ) : (
        <>
          <div
            style={{
              fontSize: '0.66rem',
              color: 'var(--wks-text-faint)',
              lineHeight: 1.6,
              marginBottom: 10,
            }}
          >
            Point this app at a <code style={inlineCode}>workspacer serve</code> on another machine
            (host or URL + its pairing token). The app restarts as a pure client — like the server's
            web app, but in this window.
          </div>
          <input
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            placeholder="server address, e.g. 100.64.1.2:7895"
            style={textInputStyle}
          />
          <input
            value={token}
            onChange={(e) => setToken(e.target.value)}
            placeholder="pairing token (from `workspacer serve`)"
            type="password"
            style={{ ...textInputStyle, marginTop: 6 }}
          />
          {error && (
            <div style={{ fontSize: '0.64rem', color: 'var(--wks-danger, #e05555)', marginTop: 6 }}>
              {error}
            </div>
          )}
          <button
            onClick={() => apply({ url, token })}
            disabled={busy || !url.trim()}
            style={{ ...primaryBtnStyle(busy || !url.trim()), marginTop: 8 }}
          >
            {busy ? 'Connecting…' : 'Connect (restarts the app)'}
          </button>
        </>
      )}
    </div>
  );
}

function DisabledState({ busy, onStart }: { busy: boolean; onStart: () => void }) {
  return (
    <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-tertiary)', lineHeight: 1.6 }}>
      <StatusPill on={false} />
      <div style={{ marginBottom: 14 }}>
        Right now the hub listens on <code style={inlineCode}>localhost</code> only — nothing off
        this machine can reach it. Start sharing to bind it to your network so a phone or another
        computer can connect.
      </div>

      <TailscaleNote />

      <button onClick={onStart} disabled={busy} style={primaryBtnStyle(busy)}>
        {busy ? 'Starting…' : 'Start sharing'}
      </button>
      <div style={{ marginTop: 8, color: 'var(--wks-text-faint)', fontSize: '0.66rem' }}>
        This restarts the hub bound to <code style={inlineCode}>0.0.0.0</code> (all interfaces) and
        generates a one-time URL with an access token. You can stop sharing again anytime.
      </div>
    </div>
  );
}

/** Status chip used by both states. */
function StatusPill({ on }: { on: boolean }) {
  const color = on ? 'var(--wks-success, #3fb950)' : 'var(--wks-text-faint, #666)';
  return (
    <div
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap: 6,
        marginBottom: 14,
        padding: '3px 10px',
        borderRadius: 'var(--wks-radius-pill)',
        background: 'var(--wks-bg-input)',
        border: '1px solid var(--wks-border-input)',
        color: on ? 'var(--wks-success, #3fb950)' : 'var(--wks-text-muted)',
        fontSize: '0.65rem',
        fontWeight: 600,
      }}
    >
      <span
        style={{
          width: 6,
          height: 6,
          borderRadius: '50%',
          background: color,
          boxShadow: on ? `0 0 5px ${color}` : 'none',
        }}
      />
      Remote sharing is {on ? 'ON' : 'OFF'}
    </div>
  );
}

/** How the connection actually reaches another device — Tailscale, in plain terms. */
function TailscaleNote() {
  return (
    <div
      style={{
        margin: '0 0 16px',
        padding: '10px 12px',
        borderRadius: 'var(--wks-radius-md)',
        background: 'var(--wks-bg-input)',
        border: '1px solid var(--wks-border-subtle, var(--wks-border-input))',
        fontSize: '0.68rem',
        color: 'var(--wks-text-tertiary)',
        lineHeight: 1.6,
      }}
    >
      <div style={{ fontWeight: 600, color: 'var(--wks-text-muted)', marginBottom: 4 }}>
        How this reaches your phone
      </div>
      Sharing exposes the hub on your machine's network address. The clean way to use it from
      anywhere is{' '}
      <a
        href="https://tailscale.com/"
        target="_blank"
        rel="noreferrer"
        style={{ color: 'var(--wks-accent, #4a9eff)' }}
      >
        Tailscale
      </a>{' '}
      — a zero-config VPN that puts all your devices on one private network (a “tailnet”) with
      stable
      <code style={inlineCode}>100.x</code> IPs. Install Tailscale on this machine and your phone,
      sign both into the same account, and the URL below works from anywhere — the traffic rides
      Tailscale's encrypted WireGuard tunnel, never the public internet. Without it, sharing only
      reaches devices on the same LAN, and the link itself isn't encrypted — so prefer the tailnet.
    </div>
  );
}

/** Render text, turning any embedded http(s) URL into a click-to-open link so
 *  Tailscale's node-specific opt-in URLs are one tap, not a copy-paste chore. */
function LinkifiedHint({ text }: { text: string }) {
  const parts = text.split(/(https?:\/\/\S+)/g);
  return (
    <>
      {parts.map((part, i) =>
        /^https?:\/\//.test(part) ? (
          <a
            key={i}
            href={part}
            onClick={(e) => {
              e.preventDefault();
              window.electronAPI.openExternalUrl?.(part);
            }}
            style={{
              color: 'var(--wks-accent, #4a9eff)',
              textDecoration: 'underline',
              cursor: 'pointer',
              wordBreak: 'break-all',
            }}
          >
            {part}
          </a>
        ) : (
          <React.Fragment key={i}>{part}</React.Fragment>
        ),
      )}
    </>
  );
}

/**
 * One-tap HTTPS via `tailscale serve`. A raw http://100.x tailnet URL is
 * encrypted on the wire but is NOT a browser "secure context", so the /m PWA
 * can't install or receive push there. When Tailscale can serve, this fronts
 * the hub at https://<node>.ts.net and the QR/URLs above switch to it.
 */
function TailscaleHttps({
  ts,
  on,
  busy,
  msg,
  onToggle,
}: {
  ts: TailscaleInfoUI | null;
  on: boolean;
  busy: boolean;
  msg: string | null;
  onToggle: (enable: boolean) => void;
}) {
  // No CLI / not logged in: nothing to offer; the general note below still
  // explains the tailnet.
  if (!ts || !ts.available) return null;
  const enableDisabled = busy || (!on && !ts.canServe);
  // Tailscale is here but HTTPS isn't on yet — surface this as a recommendation
  // (accent highlight) rather than a neutral row, so it reads as a prompt.
  const recommend = !on;
  return (
    <div
      style={{
        border: `1px solid ${recommend ? 'var(--wks-accent)' : 'var(--wks-border-subtle)'}`,
        borderRadius: 'var(--wks-radius-md)',
        padding: '10px 12px',
        marginBottom: 16,
        background: recommend
          ? 'color-mix(in srgb, var(--wks-accent) 8%, transparent)'
          : 'var(--wks-bg-elevated)',
      }}
    >
      <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div
            style={{
              fontSize: '0.72rem',
              fontWeight: 600,
              color: 'var(--wks-text-primary)',
              display: 'flex',
              alignItems: 'center',
              gap: 6,
            }}
          >
            HTTPS via Tailscale
            {on && <span style={{ color: 'var(--wks-success, #3fb950)' }}> · on</span>}
            {recommend && (
              <span
                style={{
                  fontSize: '0.56rem',
                  fontWeight: 700,
                  textTransform: 'uppercase',
                  letterSpacing: '0.04em',
                  color: 'var(--wks-accent-text, var(--wks-accent))',
                  background: 'color-mix(in srgb, var(--wks-accent) 16%, transparent)',
                  padding: '1px 6px',
                  borderRadius: 5,
                }}
              >
                Recommended
              </span>
            )}
          </div>
          <div
            style={{
              fontSize: '0.64rem',
              color: 'var(--wks-text-faint)',
              marginTop: 2,
              lineHeight: 1.5,
            }}
          >
            {on && ts.magicName ? (
              <>
                Serving at <code style={inlineCode}>{ts.magicName}</code> — home-screen install +
                notifications work.
              </>
            ) : (
              <>Needed to install the app and get notifications (a secure origin).</>
            )}
          </div>
        </div>
        <button
          onClick={() => onToggle(!on)}
          disabled={enableDisabled}
          style={{
            flexShrink: 0,
            ...(on ? secondaryBtnStyle : compactAccentBtnStyle(enableDisabled)),
          }}
        >
          {busy ? '…' : on ? 'Turn off' : 'Enable'}
        </button>
      </div>
      {!on && !ts.canServe && ts.hint && (
        <div
          style={{
            fontSize: '0.62rem',
            color: 'var(--wks-warning, #e0a000)',
            marginTop: 8,
            lineHeight: 1.5,
            fontFamily: 'var(--wks-font-mono)',
          }}
        >
          <LinkifiedHint text={ts.hint} />
        </div>
      )}
      {msg && (
        <div
          style={{
            fontSize: '0.62rem',
            color: 'var(--wks-danger, #e05555)',
            marginTop: 8,
            lineHeight: 1.5,
          }}
        >
          <LinkifiedHint text={msg} />
        </div>
      )}
      {on && (
        <div
          style={{
            fontSize: '0.6rem',
            color: 'var(--wks-text-faint)',
            marginTop: 8,
            lineHeight: 1.5,
          }}
        >
          Turning off clears this machine's <code style={inlineCode}>tailscale serve</code> config.
        </div>
      )}
    </div>
  );
}

function EnabledState({
  info,
  copied,
  showToken,
  busy,
  onCopy,
  onToggleToken,
  onStop,
}: {
  info: RemoteInfo;
  copied: string | null;
  showToken: boolean;
  busy: boolean;
  onCopy: (label: string, text: string) => void;
  onToggleToken: () => void;
  onStop: () => void;
}) {
  const hasApp = !!info.appUrl;
  // Prefer the full app when it's available; fall back to the lite client.
  const [mode, setMode] = useState<'app' | 'lite'>(hasApp ? 'app' : 'lite');

  // Tailscale HTTPS: when `tailscale serve` fronts the hub, we can hand out a
  // real https://<node>.ts.net URL — a secure origin, so the /m PWA can install
  // and receive push. Falls back to the plain-http tailnet URL otherwise.
  const [ts, setTs] = useState<TailscaleInfoUI | null>(null);
  const [tsBusy, setTsBusy] = useState(false);
  const [tsMsg, setTsMsg] = useState<string | null>(null);
  const refreshTs = useCallback(() => {
    window.electronAPI
      .tailscaleGetInfo?.()
      .then(setTs)
      .catch(() => setTs(null));
  }, []);
  useEffect(() => {
    refreshTs();
  }, [refreshTs]);

  const httpsOn = !!(ts && ts.serveActive && ts.magicName);
  const tokenQ = info.token ? `?token=${encodeURIComponent(info.token)}` : '';
  const remoteUrl = httpsOn ? `https://${ts!.magicName}/m${tokenQ}` : info.remoteUrl;
  const appUrl = httpsOn && hasApp ? `https://${ts!.magicName}/app/${tokenQ}` : info.appUrl;
  const activeUrl = mode === 'app' && hasApp ? appUrl : remoteUrl;

  const toggleServe = async (enable: boolean) => {
    setTsBusy(true);
    setTsMsg(null);
    try {
      const r = await window.electronAPI.tailscaleSetServe?.(enable);
      if (r && !r.ok) setTsMsg(r.hint || r.error || 'Tailscale command failed');
    } catch {
      setTsMsg('Tailscale command failed');
    } finally {
      setTsBusy(false);
      refreshTs();
    }
  };

  return (
    <div>
      <StatusPill on={true} />

      {/* Full app vs mobile client. The full app is the real renderer served at
          /app; mobile is the mobile-first single-page client served at /m. */}
      {hasApp ? (
        <div style={{ display: 'flex', gap: 4, marginBottom: 14 }}>
          <ModeTab
            active={mode === 'app'}
            onClick={() => setMode('app')}
            title="The full Workspacer UI, in the browser"
          >
            Full app
          </ModeTab>
          <ModeTab
            active={mode === 'lite'}
            onClick={() => setMode('lite')}
            title="Mobile-first client: fleet, decisions, and chat"
          >
            Mobile
          </ModeTab>
        </div>
      ) : (
        <div
          style={{
            fontSize: '0.66rem',
            color: 'var(--wks-text-faint)',
            marginBottom: 12,
            lineHeight: 1.5,
          }}
        >
          Showing the mobile client. Build the full app with{' '}
          <code style={inlineCode}>npm run build:renderer:web</code> and restart to share the full
          UI.
        </div>
      )}

      {/* QR — the fast path. White quiet-zone box so it scans on any theme. */}
      <div style={{ display: 'flex', justifyContent: 'center', marginBottom: 14 }}>
        <div
          style={{
            background: '#fff',
            padding: 12,
            borderRadius: 'var(--wks-radius-md)',
            lineHeight: 0,
          }}
        >
          <QRCodeSVG
            value={activeUrl}
            size={188}
            level="M"
            marginSize={0}
            bgColor="#ffffff"
            fgColor="#000000"
          />
        </div>
      </div>
      <div
        style={{
          textAlign: 'center',
          fontSize: '0.68rem',
          color: 'var(--wks-text-muted)',
          marginBottom: 16,
        }}
      >
        Scan with your phone's camera to open{' '}
        {mode === 'app' && hasApp ? 'the full app' : 'the mobile client'}.
      </div>

      <TailscaleHttps ts={ts} on={httpsOn} busy={tsBusy} msg={tsMsg} onToggle={toggleServe} />

      <CopyRow
        label="Connection URL"
        value={activeUrl}
        display={activeUrl}
        copied={copied === 'url'}
        onCopy={() => onCopy('url', activeUrl)}
      />

      <CopyRow
        label="Token"
        value={info.token}
        display={showToken ? info.token : '•'.repeat(Math.min(24, info.token.length || 8))}
        copied={copied === 'token'}
        onCopy={() => onCopy('token', info.token)}
        extra={
          <button
            onClick={onToggleToken}
            title={showToken ? 'Hide token' : 'Show token'}
            style={iconBtnStyle}
          >
            {showToken ? <EyeOff size={13} /> : <Eye size={13} />}
          </button>
        }
      />

      <div
        style={{
          marginTop: 14,
          paddingTop: 12,
          borderTop: '1px solid var(--wks-border-subtle)',
          fontSize: '0.68rem',
          color: 'var(--wks-text-faint)',
          lineHeight: 1.6,
          marginBottom: 14,
        }}
      >
        The URL already includes the token, so anyone who can open it gets full control. Only share
        it over your trusted tailnet — see below.
      </div>

      <TailscaleNote />

      <button onClick={onStop} disabled={busy} style={dangerBtnStyle(busy)}>
        {busy ? 'Stopping…' : 'Stop sharing'}
      </button>
    </div>
  );
}

function CopyRow({
  label,
  display,
  copied,
  onCopy,
  extra,
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
      <div style={{ fontSize: '0.62rem', color: 'var(--wks-text-muted)', marginBottom: 4 }}>
        {label}
      </div>
      <div style={{ display: 'flex', gap: 6, alignItems: 'stretch' }}>
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
            display: 'flex',
            alignItems: 'center',
          }}
        >
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

function ModeTab({
  active,
  onClick,
  title,
  children,
}: {
  active: boolean;
  onClick: () => void;
  title?: string;
  children: React.ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      title={title}
      style={{
        flex: 1,
        cursor: 'pointer',
        fontSize: '0.72rem',
        fontWeight: 600,
        fontFamily: 'inherit',
        padding: '6px 10px',
        borderRadius: 6,
        background: active ? 'var(--wks-accent-soft, var(--wks-bg-input))' : 'transparent',
        color: active ? 'var(--wks-text-primary)' : 'var(--wks-text-muted)',
        border: `1px solid ${active ? 'var(--wks-accent, #4a9eff)' : 'var(--wks-border-input)'}`,
      }}
    >
      {children}
    </button>
  );
}

const primaryBtnStyle = (disabled: boolean): React.CSSProperties => ({
  width: '100%',
  boxSizing: 'border-box',
  fontSize: '0.8rem',
  fontWeight: 600,
  fontFamily: 'inherit',
  cursor: disabled ? 'default' : 'pointer',
  background: disabled ? 'var(--wks-bg-input)' : 'var(--wks-accent, #4a9eff)',
  color: disabled ? 'var(--wks-text-faint)' : 'var(--wks-text-on-accent, #fff)',
  border: 'none',
  borderRadius: 6,
  padding: '9px 14px',
});

// Compact accent button for the inline HTTPS row — same footprint as the
// secondary "Turn off" button so the row doesn't jump between states. (The
// full-width primaryBtnStyle is for the standalone Start/Stop actions.)
const compactAccentBtnStyle = (disabled: boolean): React.CSSProperties => ({
  flexShrink: 0,
  fontSize: '0.78rem',
  fontWeight: 600,
  fontFamily: 'inherit',
  cursor: disabled ? 'default' : 'pointer',
  background: disabled ? 'var(--wks-bg-input)' : 'var(--wks-accent, #4a9eff)',
  color: disabled ? 'var(--wks-text-faint)' : 'var(--wks-text-on-accent, #fff)',
  border: 'none',
  borderRadius: 4,
  padding: '6px 14px',
});

const dangerBtnStyle = (disabled: boolean): React.CSSProperties => ({
  width: '100%',
  boxSizing: 'border-box',
  fontSize: '0.8rem',
  fontWeight: 600,
  fontFamily: 'inherit',
  cursor: disabled ? 'default' : 'pointer',
  background: 'transparent',
  color: disabled ? 'var(--wks-text-faint)' : 'var(--wks-danger, #e05555)',
  border: `1px solid ${disabled ? 'var(--wks-border-input)' : 'var(--wks-danger, #e05555)'}`,
  borderRadius: 6,
  padding: '9px 14px',
});

const textInputStyle: React.CSSProperties = {
  width: '100%',
  boxSizing: 'border-box',
  fontSize: '0.72rem',
  fontFamily: 'var(--wks-font-mono, monospace)',
  color: 'var(--wks-text-primary)',
  background: 'var(--wks-bg-base)',
  border: '1px solid var(--wks-border-input)',
  borderRadius: 4,
  padding: '7px 9px',
  outline: 'none',
};

const inlineCode: React.CSSProperties = {
  fontFamily: 'var(--wks-font-mono, monospace)',
  background: 'var(--wks-bg-base)',
  padding: '1px 4px',
  borderRadius: 3,
};

const iconBtnStyle: React.CSSProperties = {
  display: 'inline-flex',
  alignItems: 'center',
  justifyContent: 'center',
  cursor: 'pointer',
  flexShrink: 0,
  background: 'var(--wks-bg-input)',
  color: 'var(--wks-text-tertiary)',
  border: '1px solid var(--wks-border-input)',
  borderRadius: 4,
  padding: '0 9px',
};

const secondaryBtnStyle: React.CSSProperties = {
  fontSize: '0.78rem',
  fontFamily: 'inherit',
  cursor: 'pointer',
  background: 'transparent',
  color: 'var(--wks-text-tertiary)',
  border: '1px solid var(--wks-border-input)',
  borderRadius: 4,
  padding: '6px 14px',
};

export default RemoteShareDialog;
