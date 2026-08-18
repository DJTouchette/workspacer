import React, { useCallback, useEffect, useState } from 'react';
import { Check, Copy, Eye, EyeOff, KeyRound } from 'lucide-react';
import type { RemoteTokenRecord, RemoteTokenScope } from '../../../main/shared/ipcTypes';

/**
 * "Linked machines" — the federation peers editor inside the Remote Control
 * dialog. Two directions, one section:
 *
 * - Outbound: the peers THIS machine's hub dials (`~/.config/workspacer/
 *   peers.json`). Listed with live link status (federationPeers) merged over
 *   the stored config (federationPeersConfig — tokens redacted to hasToken).
 *   Add/remove save the whole set via federationSavePeersConfig; entries the
 *   renderer didn't touch are re-sent with `token: undefined`, which keeps
 *   their stored token (secrets never round-trip through this process). A save
 *   restarts the hub so the new links dial.
 *
 * - Inbound ("Link this machine"): the OTHER PC adds this one. We hand over
 *   the two things it needs — this hub's bus URL and a scoped token minted
 *   with the same remote-token IPC the pairing QR uses.
 *
 * When federationPeersConfig() resolves null (the web mirror can't read the
 * hub machine's peers.json) the section renders read-only from live status.
 */

interface ConfiguredPeer {
  name: string;
  url: string;
  hasToken: boolean;
}

interface LivePeer {
  name: string;
  connected: boolean;
  /** Unix milliseconds (federation.peers encoding); 0/absent = never. */
  lastSeen?: number;
}

/** Twin of main's validation (federationPeersConfig.ts ← federation.go). */
const PEER_NAME_RE = /^[A-Za-z0-9_-]+$/;

const LINK_SCOPES: Array<{ scope: RemoteTokenScope; label: string; hint: string }> = [
  { scope: 'view', label: 'Read-only', hint: 'The other PC can watch this fleet, not control it.' },
  {
    scope: 'triage',
    label: 'Triage',
    hint: 'Approve, answer, chat, interrupt — no spawn, git, or terminal here.',
  },
  {
    scope: 'operator',
    label: 'Full control',
    hint: 'The other PC can do everything on this machine, including spawning agents.',
  },
];

function formatLastSeen(ms?: number): string {
  if (!ms) return 'never connected';
  const delta = Date.now() - ms;
  if (delta < 60_000) return 'last seen just now';
  if (delta < 3_600_000) return `last seen ${Math.floor(delta / 60_000)}m ago`;
  if (delta < 86_400_000) return `last seen ${Math.floor(delta / 3_600_000)}h ago`;
  return `last seen ${new Date(ms).toLocaleDateString(undefined, {
    month: 'short',
    day: 'numeric',
  })}`;
}

const LinkedMachinesSection: React.FC<{
  /** This hub's bare bus URL from getRemoteShareInfo(), when known. */
  busUrl?: string;
  /** Whether sharing is on — while off, the hub binds loopback and no other
   *  machine can reach it, so the inbound panel warns instead of pretending. */
  sharingOn: boolean;
}> = ({ busUrl, sharingOn }) => {
  const [config, setConfig] = useState<ConfiguredPeer[] | null>(null);
  const [readOnly, setReadOnly] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [live, setLive] = useState<LivePeer[]>([]);
  const [busy, setBusy] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const [cfg, peers] = await Promise.all([
        window.electronAPI.federationPeersConfig?.() ?? Promise.resolve(null),
        window.electronAPI.federationPeers?.() ?? Promise.resolve([]),
      ]);
      setReadOnly(cfg === null);
      setConfig(cfg);
      setLive(peers || []);
    } catch {
      setReadOnly(true);
      setConfig(null);
    } finally {
      setLoaded(true);
    }
  }, []);

  useEffect(() => {
    refresh();
    // Live transitions arrive as hub.peer.* bus events — refresh status on
    // those rather than polling (per the federationPeers contract).
    const off = window.electronAPI.onHubEvent?.((ev) => {
      if (ev.type === 'hub.peer.connected' || ev.type === 'hub.peer.disconnected') refresh();
    });
    return () => off?.();
  }, [refresh]);

  /** Replace the peer set wholesale; untouched entries keep stored tokens. */
  const save = async (
    next: Array<{ name: string; url: string; token?: string }>,
  ): Promise<boolean> => {
    setBusy(true);
    setSaveError(null);
    try {
      const r = await window.electronAPI.federationSavePeersConfig?.(next);
      if (!r?.ok) {
        setSaveError(r?.error || 'Could not save linked machines.');
        return false;
      }
      await refresh();
      return true;
    } catch {
      setSaveError('Could not save linked machines.');
      return false;
    } finally {
      setBusy(false);
    }
  };

  const removePeer = (name: string) => {
    if (!config) return;
    // token undefined on every kept row = keep its stored token.
    void save(config.filter((p) => p.name !== name).map((p) => ({ name: p.name, url: p.url })));
  };

  const liveByName = new Map(live.map((p) => [p.name, p]));
  const rows: Array<{ name: string; url?: string; hasToken?: boolean; live?: LivePeer }> = readOnly
    ? live.map((p) => ({ name: p.name, live: p }))
    : (config || []).map((p) => ({ ...p, live: liveByName.get(p.name) }));

  return (
    <div>
      {/* Rendered as the Remote Control dialog's Machines tab — the tab strip
          is the header, so no section border or icon of its own. */}
      <div
        style={{
          fontSize: '0.78rem',
          fontWeight: 600,
          color: 'var(--wks-text-primary)',
          marginBottom: 4,
        }}
      >
        Linked machines
      </div>
      <div
        style={{
          fontSize: '0.69rem',
          color: 'var(--wks-text-faint)',
          lineHeight: 1.5,
          marginBottom: 10,
        }}
      >
        Link another PC's hub and its agents appear in this fleet. Each link is one-way: this
        machine connects out to the peer using a token minted over there.
      </div>

      {!loaded && (
        <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-faint)' }}>Loading…</div>
      )}

      {loaded && readOnly && (
        <div
          style={{
            fontSize: '0.69rem',
            color: 'var(--wks-text-tertiary)',
            lineHeight: 1.5,
            marginBottom: 10,
          }}
        >
          Linked machines are configured on the hub machine — this client only shows their status.
        </div>
      )}

      {loaded && rows.length === 0 && (
        <div
          style={{
            fontSize: '0.7rem',
            color: 'var(--wks-text-faint)',
            border: '1px dashed var(--wks-border-input)',
            borderRadius: 'var(--wks-radius-md)',
            padding: '10px 12px',
            marginBottom: 10,
          }}
        >
          No machines linked yet.
        </div>
      )}

      {loaded && rows.length > 0 && (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 5, marginBottom: 10 }}>
          {rows.map((p) => (
            <PeerRow
              key={p.name}
              name={p.name}
              url={p.url}
              hasToken={p.hasToken}
              live={p.live}
              busy={busy}
              onRemove={readOnly ? undefined : () => removePeer(p.name)}
            />
          ))}
        </div>
      )}

      {saveError && (
        <div style={{ fontSize: '0.67rem', color: 'var(--wks-error)', marginBottom: 10 }}>
          {saveError}
        </div>
      )}
      {busy && (
        <div style={{ fontSize: '0.67rem', color: 'var(--wks-text-muted)', marginBottom: 10 }}>
          Saving — restarting the hub so the new links take effect…
        </div>
      )}

      {loaded && !readOnly && (
        <AddPeerForm
          existing={(config || []).map((p) => ({ name: p.name, url: p.url }))}
          busy={busy}
          onAdd={save}
        />
      )}

      {loaded && <LinkThisMachine busUrl={busUrl} sharingOn={sharingOn} />}
    </div>
  );
};

function PeerRow({
  name,
  url,
  hasToken,
  live,
  busy,
  onRemove,
}: {
  name: string;
  url?: string;
  hasToken?: boolean;
  live?: LivePeer;
  busy: boolean;
  onRemove?: () => void;
}) {
  const connected = !!live?.connected;
  const dotColor = connected ? 'var(--wks-success)' : 'var(--wks-warning)';
  const status = connected ? 'connected' : formatLastSeen(live?.lastSeen);
  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 8,
        border: '1px solid var(--wks-border-input)',
        borderRadius: 6,
        background: 'var(--wks-bg-base)',
        padding: '6px 8px',
      }}
    >
      <span
        title={status}
        style={{
          flexShrink: 0,
          width: 7,
          height: 7,
          borderRadius: '50%',
          background: dotColor,
          boxShadow: connected ? `0 0 5px ${dotColor}` : 'none',
        }}
      />
      <div style={{ flex: 1, minWidth: 0 }}>
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 6,
            fontSize: '0.69rem',
            color: 'var(--wks-text-secondary)',
            minWidth: 0,
          }}
        >
          <span style={{ fontWeight: 700, color: 'var(--wks-text-primary)' }}>{name}</span>
          <span style={{ color: connected ? 'var(--wks-success)' : 'var(--wks-warning)' }}>
            {status}
          </span>
          {hasToken && (
            <span
              title="A link token is stored for this peer"
              style={{ display: 'inline-flex', alignItems: 'center', color: 'var(--wks-text-muted)' }}
            >
              <KeyRound size={11} strokeWidth={2} />
            </span>
          )}
        </div>
        {url && (
          <div
            style={{
              fontSize: '0.62rem',
              color: 'var(--wks-text-faint)',
              fontFamily: 'var(--wks-font-mono)',
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
            }}
            title={url}
          >
            {url}
          </div>
        )}
      </div>
      {onRemove && (
        <button
          onClick={onRemove}
          disabled={busy}
          title={`Unlink ${name}`}
          style={{
            flexShrink: 0,
            fontSize: '0.66rem',
            fontWeight: 700,
            fontFamily: 'inherit',
            cursor: busy ? 'default' : 'pointer',
            color: busy ? 'var(--wks-text-faint)' : 'var(--wks-error)',
            background: 'transparent',
            border: '1px solid var(--wks-border-input)',
            borderRadius: 4,
            padding: '3px 7px',
          }}
        >
          Remove
        </button>
      )}
    </div>
  );
}

/** Add one peer. Validation mirrors main's (which mirrors the hub's), so the
 *  inline error is the same message a rejected save would return. */
function AddPeerForm({
  existing,
  busy,
  onAdd,
}: {
  existing: Array<{ name: string; url: string }>;
  busy: boolean;
  onAdd: (next: Array<{ name: string; url: string; token?: string }>) => Promise<boolean>;
}) {
  const [name, setName] = useState('');
  const [url, setUrl] = useState('');
  const [token, setToken] = useState('');
  const [error, setError] = useState<string | null>(null);

  const validate = (): string | null => {
    const n = name.trim();
    const u = url.trim();
    if (!n || !u) return 'Name and bus URL are required.';
    if (!PEER_NAME_RE.test(n)) return 'Name: use letters, digits, - or _ only.';
    if (!u.startsWith('ws://') && !u.startsWith('wss://'))
      return 'URL must start with ws:// or wss:// (e.g. ws://100.64.1.2:7895/bus).';
    if (existing.some((p) => p.name === n)) return `A machine named "${n}" is already linked.`;
    return null;
  };

  const submit = async () => {
    const err = validate();
    if (err) {
      setError(err);
      return;
    }
    setError(null);
    // Existing rows go back with token undefined → main keeps their stored
    // tokens; only the new row carries a secret, and only this once.
    const t = token.trim();
    const ok = await onAdd([
      ...existing.map((p) => ({ name: p.name, url: p.url })),
      { name: name.trim(), url: url.trim(), ...(t ? { token: t } : {}) },
    ]);
    if (ok) {
      setName('');
      setUrl('');
      setToken('');
    }
  };

  return (
    <div style={{ marginBottom: 4 }}>
      <div style={{ fontSize: '0.66rem', color: 'var(--wks-text-muted)', marginBottom: 6 }}>
        Link a machine
      </div>
      <div style={{ display: 'flex', gap: 6 }}>
        <input
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="name (e.g. work-pc)"
          style={{ ...fieldStyle, flex: '0 0 34%' }}
        />
        <input
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          placeholder="ws://100.64.1.2:7895/bus"
          style={{ ...fieldStyle, flex: 1 }}
        />
      </div>
      <input
        value={token}
        onChange={(e) => setToken(e.target.value)}
        placeholder="link token (minted on that machine — see below)"
        type="password"
        style={{ ...fieldStyle, width: '100%', boxSizing: 'border-box', marginTop: 6 }}
      />
      {error && (
        <div style={{ fontSize: '0.67rem', color: 'var(--wks-error)', marginTop: 6 }}>{error}</div>
      )}
      <button
        onClick={submit}
        disabled={busy || !name.trim() || !url.trim()}
        style={{
          marginTop: 8,
          width: '100%',
          boxSizing: 'border-box',
          fontSize: '0.74rem',
          fontWeight: 600,
          fontFamily: 'inherit',
          cursor: busy || !name.trim() || !url.trim() ? 'default' : 'pointer',
          background:
            busy || !name.trim() || !url.trim() ? 'var(--wks-bg-input)' : 'var(--wks-accent)',
          color:
            busy || !name.trim() || !url.trim()
              ? 'var(--wks-text-faint)'
              : 'var(--wks-text-on-accent)',
          border: 'none',
          borderRadius: 6,
          padding: '7px 12px',
        }}
      >
        {busy ? 'Linking…' : 'Link machine'}
      </button>
    </div>
  );
}

/** The inbound direction: what the OTHER PC needs to add THIS machine. */
function LinkThisMachine({ busUrl, sharingOn }: { busUrl?: string; sharingOn: boolean }) {
  const [scope, setScope] = useState<RemoteTokenScope>('operator');
  const [minted, setMinted] = useState<RemoteTokenRecord | null>(null);
  const [minting, setMinting] = useState(false);
  const [mintError, setMintError] = useState<string | null>(null);
  const [showToken, setShowToken] = useState(false);
  const [copied, setCopied] = useState<string | null>(null);

  const canMint = !!window.electronAPI.remoteTokenGetOrCreate;
  const shownUrl = busUrl || 'ws://<this-host>:7895/bus';
  const scopeHint = LINK_SCOPES.find((s) => s.scope === scope)?.hint || '';

  const copy = async (label: string, text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(label);
      setTimeout(() => setCopied((c) => (c === label ? null : c)), 1500);
    } catch {
      /* clipboard blocked */
    }
  };

  const mint = async () => {
    if (!window.electronAPI.remoteTokenGetOrCreate) return;
    setMinting(true);
    setMintError(null);
    try {
      const rec = await window.electronAPI.remoteTokenGetOrCreate(scope, `Machine link: ${scope}`);
      setMinted(rec);
    } catch (err) {
      setMintError(err instanceof Error ? err.message : String(err));
    } finally {
      setMinting(false);
    }
  };

  return (
    <details style={{ marginTop: 12 }}>
      <summary
        style={{
          cursor: 'pointer',
          fontSize: '0.7rem',
          fontWeight: 700,
          color: 'var(--wks-text-muted)',
          listStylePosition: 'inside',
        }}
      >
        Link this machine from another PC
      </summary>
      <div
        style={{
          fontSize: '0.69rem',
          color: 'var(--wks-text-faint)',
          lineHeight: 1.5,
          margin: '8px 0 10px',
        }}
      >
        On the other PC, add this machine as a linked machine using this bus URL and a token minted
        here.
      </div>

      {!sharingOn && (
        <div
          style={{
            marginBottom: 10,
            padding: '8px 10px',
            borderRadius: 'var(--wks-radius-md)',
            background: 'color-mix(in srgb, var(--wks-warning) 8%, transparent)',
            border: '1px solid color-mix(in srgb, var(--wks-warning) 30%, transparent)',
            fontSize: '0.67rem',
            color: 'var(--wks-text-secondary)',
            lineHeight: 1.5,
          }}
        >
          Sharing is off, so this hub only listens on localhost — turn on sharing above before the
          other PC can connect.
        </div>
      )}

      <SmallCopyRow
        label="This machine's bus URL"
        display={shownUrl}
        copied={copied === 'bus'}
        onCopy={() => copy('bus', shownUrl)}
      />
      {!busUrl && (
        <div style={{ fontSize: '0.64rem', color: 'var(--wks-text-faint)', marginBottom: 8 }}>
          Replace <code style={inlineCode}>&lt;this-host&gt;</code> with this machine's tailnet or
          LAN address.
        </div>
      )}

      {canMint && (
        <>
          <div
            style={{
              display: 'flex',
              gap: 6,
              alignItems: 'center',
              marginTop: 4,
              marginBottom: 4,
            }}
          >
            <select
              value={scope}
              onChange={(e) => {
                setScope(e.target.value as RemoteTokenScope);
                setMinted(null);
              }}
              style={{
                fontSize: '0.7rem',
                fontFamily: 'inherit',
                color: 'var(--wks-text-primary)',
                background: 'var(--wks-bg-input)',
                border: '1px solid var(--wks-border-input)',
                borderRadius: 4,
                padding: '5px 6px',
              }}
            >
              {LINK_SCOPES.map((s) => (
                <option key={s.scope} value={s.scope}>
                  {s.label}
                </option>
              ))}
            </select>
            <button
              onClick={mint}
              disabled={minting}
              style={{
                flex: 1,
                fontSize: '0.7rem',
                fontWeight: 600,
                fontFamily: 'inherit',
                cursor: minting ? 'default' : 'pointer',
                background: minting ? 'var(--wks-bg-input)' : 'var(--wks-accent)',
                color: minting ? 'var(--wks-text-faint)' : 'var(--wks-text-on-accent)',
                border: 'none',
                borderRadius: 4,
                padding: '6px 10px',
              }}
            >
              {minting ? 'Minting…' : minted ? 'Token ready' : 'Mint link token'}
            </button>
          </div>
          <div
            style={{
              fontSize: '0.64rem',
              color: 'var(--wks-text-faint)',
              lineHeight: 1.5,
              marginBottom: 8,
            }}
          >
            {scopeHint} The token is the ceiling on everything the other PC can do here.
          </div>
          {mintError && (
            <div style={{ fontSize: '0.67rem', color: 'var(--wks-error)', marginBottom: 8 }}>
              {mintError}
            </div>
          )}
          {minted && (
            <SmallCopyRow
              label="Link token"
              display={
                showToken ? minted.token : '•'.repeat(Math.min(24, minted.token.length || 8))
              }
              copied={copied === 'token'}
              onCopy={() => copy('token', minted.token)}
              extra={
                <button
                  onClick={() => setShowToken((s) => !s)}
                  title={showToken ? 'Hide token' : 'Show token'}
                  style={iconBtnStyle}
                >
                  {showToken ? <EyeOff size={13} /> : <Eye size={13} />}
                </button>
              }
            />
          )}
        </>
      )}
      {!canMint && (
        <div style={{ fontSize: '0.67rem', color: 'var(--wks-text-faint)', marginTop: 6 }}>
          Mint the link token on the hub machine itself (its Remote control dialog, or
          <code style={inlineCode}>workspacer token create</code>).
        </div>
      )}
    </details>
  );
}

function SmallCopyRow({
  label,
  display,
  copied,
  onCopy,
  extra,
}: {
  label: string;
  display: string;
  copied: boolean;
  onCopy: () => void;
  extra?: React.ReactNode;
}) {
  return (
    <div style={{ marginBottom: 8 }}>
      <div style={{ fontSize: '0.66rem', color: 'var(--wks-text-muted)', marginBottom: 4 }}>
        {label}
      </div>
      <div style={{ display: 'flex', gap: 6, alignItems: 'stretch' }}>
        <div
          style={{
            flex: 1,
            minWidth: 0,
            fontSize: '0.72rem',
            fontFamily: 'var(--wks-font-mono)',
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
          {copied ? <Check size={13} color="var(--wks-success)" /> : <Copy size={13} />}
        </button>
      </div>
    </div>
  );
}

const fieldStyle: React.CSSProperties = {
  minWidth: 0,
  fontSize: '0.72rem',
  fontFamily: 'var(--wks-font-mono)',
  color: 'var(--wks-text-primary)',
  background: 'var(--wks-bg-base)',
  border: '1px solid var(--wks-border-input)',
  borderRadius: 4,
  padding: '7px 9px',
  outline: 'none',
};

const inlineCode: React.CSSProperties = {
  fontFamily: 'var(--wks-font-mono)',
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

export default LinkedMachinesSection;
