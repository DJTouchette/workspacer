import React, { useEffect, useRef, useState } from 'react';
import BrowserPane from './BrowserPane';

interface PluginPaneProps {
  paneId: string;
  title: string;
  isActive: boolean;
  /** The pre-built webview URL, with the static per-plugin busToken baked in as
   *  the fallback. */
  url: string;
  hibernated?: boolean;
  /** The contributing plugin's id; present for agent-scoped panes. */
  pluginId?: string;
  /** The agent's working directory; present for agent-scoped panes. */
  cwd?: string;
}

/**
 * Wraps a plugin's webview pane with a per-pane bus-token lifecycle.
 *
 * For an agent-scoped pane (it has both a plugin id and a cwd) we mint an
 * ephemeral token confined to that agent's directory and swap it into the
 * webview URL, so the plugin reaches only that project's files — not the broader
 * scope of the static per-plugin token. The token is revoked when the pane
 * unmounts (closed, tab removed, agent terminated — every path runs the cleanup),
 * and the hub also sweeps it if the plugin unloads.
 *
 * If there's nothing to scope (a global pane) or minting is unavailable (the web
 * build, or the hub momentarily down), we render the URL as-is with its baked-in
 * static token. So the webview always loads; scoping is an upgrade, not a gate.
 */
const PluginPane: React.FC<PluginPaneProps> = ({ paneId, title, isActive, url, hibernated, pluginId, cwd }) => {
  const canScope = !!(pluginId && cwd && window.electronAPI.pluginPaneToken);
  // null = still minting (agent-scoped panes only); otherwise the URL to load.
  const [resolvedUrl, setResolvedUrl] = useState<string | null>(canScope ? null : url);
  const tokenRef = useRef<string | null>(null);

  useEffect(() => {
    if (!canScope) {
      setResolvedUrl(url);
      return;
    }
    let cancelled = false;
    window.electronAPI.pluginPaneToken!(pluginId!, cwd!)
      .then((token) => {
        if (cancelled) return;
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
      })
      .catch(() => { if (!cancelled) setResolvedUrl(url); });

    return () => {
      cancelled = true;
      if (tokenRef.current) {
        window.electronAPI.revokePluginPaneToken?.(tokenRef.current);
        tokenRef.current = null;
      }
    };
  }, [canScope, pluginId, cwd, url]);

  // Brief, only while an agent-scoped pane mints its token (a local hub round-trip).
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
    />
  );
};

export default PluginPane;
