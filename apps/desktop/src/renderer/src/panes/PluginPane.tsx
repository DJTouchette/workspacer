import React, { useEffect, useRef, useState } from 'react';
import BrowserPane from './BrowserPane';

interface PluginPaneProps {
  paneId: string;
  title: string;
  isActive: boolean;
  /** The webview URL. Carries the static per-plugin busToken as a fallback when
   *  the pane was opened locally; a pane restored from the shared layout
   *  document has none (it is redacted before publishing — see useLayoutSync)
   *  and depends on the mint below. */
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
 * If minting is unavailable (the web build, or the hub momentarily down) we
 * render the URL as-is: with its baked-in static token when the pane was opened
 * locally, and unauthenticated when it came from the shared layout — the webview
 * loads either way and reports its own bus state. Scoping is an upgrade, not a gate.
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
  const canMint = !!(pluginId && window.electronAPI.pluginPaneToken);
  // null = still minting; otherwise the URL to load.
  const [resolvedUrl, setResolvedUrl] = useState<string | null>(canMint ? null : url);
  const tokenRef = useRef<string | null>(null);

  useEffect(() => {
    if (!canMint) {
      setResolvedUrl(url);
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
          const u = new URL(url);
          u.searchParams.set('busToken', token);
          setResolvedUrl(u.toString());
        } catch {
          setResolvedUrl(url);
        }
      } else {
        setResolvedUrl(url); // mint failed → fall back to the static-token URL
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
  }, [canMint, pluginId, cwd, url]);

  // Brief, only while the pane mints its token (a local hub round-trip).
  if (resolvedUrl === null) {
    return <div style={{ width: '100%', height: '100%', background: 'var(--bg, #1e1e1e)' }} />;
  }

  return (
    <BrowserPane
      paneId={paneId}
      title={title}
      isActive={isActive}
      initialUrl={resolvedUrl || 'about:blank'}
      appMode={true}
      hibernated={hibernated}
      onUrlChange={() => {}}
      pluginId={pluginId}
    />
  );
};

export default PluginPane;
