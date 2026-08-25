import React, { useEffect, useMemo, useRef, useState } from 'react';
import BrowserPane from './BrowserPane';
import { usePluginsContext } from '../contexts/PluginsContext';
import { relocatePluginURL } from '../types/plugin';

interface PluginPaneProps {
  paneId: string;
  title: string;
  isActive: boolean;
  /** The webview URL. Carries the static per-plugin busToken as a fallback when
   *  the pane was opened locally; a pane restored from the shared layout
   *  document has none (it is redacted before publishing — see useLayoutSync)
   *  and depends on the mint below. Its ORIGIN is whatever the client that
   *  opened the pane could reach, which is not necessarily what this one can —
   *  see the rehome below. */
  url: string;
  hibernated?: boolean;
  /** The contributing plugin's id — what the pane mints its own token against. */
  pluginId?: string;
  /** The agent's working directory; present for agent-scoped panes. */
  cwd?: string;
}

/**
 * Wraps a plugin's webview pane with a per-pane bus-token lifecycle.
 *
 * Every pane that knows its plugin mints its own ephemeral token here and swaps
 * it into the webview URL. For an agent-scoped pane (it also has a cwd) the
 * token is confined to that agent's directory, so the plugin reaches only that
 * project's files rather than the broader scope of the static per-plugin token.
 * For a global pane the mint carries no dynamic binding and lands on the same
 * grants as the static token — worth doing anyway, because it means the pane
 * works from a URL with no credential in it, which is what lets the shared
 * layout document be published token-free (see useLayoutSync). The token is
 * revoked when the pane unmounts (closed, tab removed, agent terminated — every
 * path runs the cleanup), and the hub also sweeps it if the plugin unloads.
 *
 * If minting is unavailable (the hub momentarily down) we render the URL as-is:
 * with its baked-in static token when the pane was opened locally, and
 * unauthenticated when it came from the shared layout — the guest loads either
 * way and reports its own bus state. Scoping is an upgrade, not a gate.
 *
 * ── Whose address is that? ────────────────────────────────────────────────
 *
 * A pane URL is baked at open time and persisted in the shared layout, so a
 * pane opened on the desktop carries THAT machine's loopback. Restored in a
 * browser somewhere else, `http://127.0.0.1:9211` is the viewer's own computer.
 * So the URL is rehomed against the live manifest — which carries the bases
 * whoever fetched it could actually reach — before anything is framed, and a
 * plugin whose UI this client cannot reach at all renders an explanation
 * instead of a frame pointed somewhere wrong.
 *
 * On /app the guest is a sandboxed `<iframe>` rather than a `<webview>` (see
 * `lib/guestFrame.ts`). Minting itself works there — `webBackend.pluginPaneToken`
 * goes over the hub's guarded HTTP route — but whether the plugin can USE the
 * token depends on where its UI is served from: cross-origin with /app (a
 * sidecar, or a hub whose second `--plugin-origin` spelling this client
 * resolved) keeps its bus link, while a same-origin hub-served `ui` plugin is
 * framed opaque and loses it. BrowserPane says so on the pane rather than
 * leaving it looking broken.
 */
const PluginPane: React.FC<PluginPaneProps> = ({
  paneId,
  title,
  isActive,
  url,
  hibernated,
  pluginId,
  cwd,
}) => {
  const { plugins, frameOrigin } = usePluginsContext();
  const manifest = useMemo(
    () => (pluginId ? plugins.find((p) => p.id === pluginId) : undefined),
    [plugins, pluginId],
  );
  /** The URL as reachable from HERE (see the rehome note above). */
  const homeUrl = useMemo(
    () => relocatePluginURL(url, manifest, frameOrigin),
    [url, manifest, frameOrigin],
  );

  const canMint = !!(pluginId && homeUrl && window.electronAPI.pluginPaneToken);
  // null = still minting; otherwise the URL to load ('' = nowhere to load from).
  const [resolvedUrl, setResolvedUrl] = useState<string | null>(canMint ? null : homeUrl);
  const tokenRef = useRef<string | null>(null);

  useEffect(() => {
    if (!canMint) {
      setResolvedUrl(homeUrl);
      return;
    }
    let cancelled = false;
    // Deadline on the mint: main's hub fetch now aborts at 3s, but if anything
    // in the chain still wedges (hub restarting mid plugin-op) the pane must
    // fall back to the static-token URL, never sit blank forever. A mint that
    // resolves AFTER the deadline (or after unmount) is revoked immediately —
    // an unrecorded token would otherwise outlive the pane until the hub
    // sweeps it on plugin unload.
    let settled = false;
    const finish = (token: string | null) => {
      if (settled || cancelled) {
        if (token && token !== tokenRef.current) {
          window.electronAPI.revokePluginPaneToken?.(token);
        }
        return;
      }
      settled = true;
      if (token) {
        tokenRef.current = token;
        try {
          const u = new URL(homeUrl);
          u.searchParams.set('busToken', token);
          setResolvedUrl(u.toString());
        } catch {
          setResolvedUrl(homeUrl);
        }
      } else {
        setResolvedUrl(homeUrl); // mint failed → fall back to the static-token URL
      }
    };
    const deadline = setTimeout(() => finish(null), 4000);
    window.electronAPI.pluginPaneToken!(pluginId!, cwd)
      .then((token) => {
        clearTimeout(deadline);
        finish(token);
      })
      .catch(() => {
        clearTimeout(deadline);
        finish(null);
      });

    return () => {
      cancelled = true;
      if (tokenRef.current) {
        window.electronAPI.revokePluginPaneToken?.(tokenRef.current);
        tokenRef.current = null;
      }
    };
  }, [canMint, pluginId, cwd, homeUrl]);

  // Brief, only while the pane mints its token (a local hub round-trip).
  if (resolvedUrl === null) {
    return <div style={{ width: '100%', height: '100%', background: 'var(--bg, #1e1e1e)' }} />;
  }

  // Nowhere to load from. Never fall through to BrowserPane, which would
  // normalize an empty URL into the configured homepage — a plugin pane
  // silently showing a search engine is exactly the kind of quiet wrong answer
  // this pane exists to avoid.
  if (!resolvedUrl) {
    return <PluginUnreachable title={title} kind={unreachableKind(manifest, plugins.length)} />;
  }

  return (
    <BrowserPane
      paneId={paneId}
      title={title}
      isActive={isActive}
      initialUrl={resolvedUrl}
      appMode={true}
      hibernated={hibernated}
      onUrlChange={() => {}}
      pluginId={pluginId}
    />
  );
};

type UnreachableKind = 'sidecar' | 'unknown-plugin' | 'no-address';

/** Why there is no address, from what the live manifest does and doesn't say. */
function unreachableKind(
  manifest: { server?: { port?: number } } | undefined,
  known: number,
): UnreachableKind {
  if (manifest?.server?.port) return 'sidecar';
  if (!manifest && known > 0) return 'unknown-plugin';
  return 'no-address';
}

const REASONS: Record<UnreachableKind, string> = {
  // The honest one, and the common one on /app: sidecar plugins serve their own
  // UI from a port on the hub machine's loopback. A browser on a different
  // machine cannot reach that address at all, and the address it WOULD reach is
  // its own computer.
  sidecar:
    'This plugin serves its interface from its own sidecar server on the workspacer host, reachable only from that machine. Open this pane in the desktop app, or in a browser on the host itself.',
  'unknown-plugin':
    'This pane belongs to a plugin that is not installed on the workspacer you are connected to. Install it there, or close this pane.',
  'no-address':
    'This client was not told where this plugin serves its interface. That usually means the token it is using is too narrow to read the full plugin manifest.',
};

const PluginUnreachable: React.FC<{ title: string; kind: UnreachableKind }> = ({ title, kind }) => (
  <div
    data-testid="plugin-unreachable"
    style={{
      width: '100%',
      height: '100%',
      display: 'flex',
      flexDirection: 'column',
      alignItems: 'center',
      justifyContent: 'center',
      gap: 8,
      padding: 24,
      textAlign: 'center',
      background: 'var(--wks-bg-base)',
      color: 'var(--wks-text-muted)',
    }}
  >
    <span style={{ fontSize: '1.6rem', opacity: 0.6 }}>{'\u{1F50C}'}</span>
    <span style={{ fontSize: '0.78rem', color: 'var(--wks-text-secondary)' }}>
      {title} can&rsquo;t be shown here.
    </span>
    <span style={{ fontSize: '0.68rem', maxWidth: 380, lineHeight: 1.5 }}>{REASONS[kind]}</span>
  </div>
);

export default PluginPane;
