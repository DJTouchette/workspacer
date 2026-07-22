import React, { useCallback, useEffect, useState } from 'react';
import { usePlugins } from '../hooks/usePlugins';
import PluginInstallDialog from '../components/PluginInstallDialog';
import ExamplesGalleryDialog from '../components/ExamplesGalleryDialog';
import PluginCatalogDialog from '../components/PluginCatalogDialog';
import { pluginRequirement } from '../types/plugin';
import type { PluginUpdateStatus } from '../types/plugin';
import { hasSensitivePermission } from '../lib/pluginPermissions';
import { PluginPermissions } from '../components/plugin/PluginPermissions';
import { Blocks, AlertTriangle, RefreshCw } from '../components/icons';

interface SidecarStatus {
  state: string;
  err?: string;
}

/** Latest supervisor state (+ last error) per plugin id, from `sidecar.*` events. */
function useSidecarStates(): Record<string, SidecarStatus> {
  const [states, setStates] = useState<Record<string, SidecarStatus>>({});
  useEffect(() => {
    const off = window.electronAPI.onHubEvent?.((ev) => {
      if (!ev.type?.startsWith('sidecar.')) return;
      const d = ev.data as { name?: string; state?: string; err?: string } | undefined;
      if (d?.name && d?.state) {
        setStates((prev) => ({
          ...prev,
          [d.name as string]: { state: d.state as string, err: d.err },
        }));
      }
    });
    return () => off?.();
  }, []);
  return states;
}

function stateColor(s: string | undefined): string {
  switch (s) {
    case 'healthy':
    case 'running':
      return 'var(--wks-success)';
    case 'unhealthy':
      return 'var(--wks-warning)';
    case 'crashed':
      return 'var(--wks-error)';
    case 'stopped':
    case 'disabled':
      return 'var(--wks-text-faint)';
    default:
      return 'var(--wks-text-faint)';
  }
}

/** Shared style for the small per-plugin action buttons (Reinstall / Enable / Remove). */
function actionBtn(busy: boolean, danger?: boolean): React.CSSProperties {
  return {
    fontSize: '0.7rem',
    fontFamily: 'inherit',
    cursor: busy ? 'default' : 'pointer',
    background: 'transparent',
    color: danger ? 'var(--wks-error)' : 'var(--wks-text-secondary)',
    border: '1px solid var(--wks-border-input)',
    borderRadius: 4,
    padding: '3px 10px',
    opacity: busy ? 0.5 : 1,
  };
}

/** Filled accent style for the "Update available" button — the one action we
 *  want to stand out, shown only when a newer version actually exists. */
function updateBtn(busy: boolean): React.CSSProperties {
  return {
    fontSize: '0.7rem',
    fontFamily: 'inherit',
    fontWeight: 600,
    cursor: busy ? 'default' : 'pointer',
    background: 'var(--wks-accent)',
    color: 'var(--wks-text-on-accent)',
    border: '1px solid var(--wks-accent)',
    borderRadius: 4,
    padding: '3px 10px',
    opacity: busy ? 0.5 : 1,
  };
}

const PluginsManagerPane: React.FC<{ title?: string }> = () => {
  const { plugins } = usePlugins();
  const sidecar = useSidecarStates();
  const [showInstall, setShowInstall] = useState(false);
  const [showExamples, setShowExamples] = useState(false);
  const [showCatalog, setShowCatalog] = useState(false);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [permsOpen, setPermsOpen] = useState<Set<string>>(new Set());
  // Per-plugin update status (id → status), populated by an on-demand check
  // against each plugin's install source. Empty until the first check returns.
  const [updates, setUpdates] = useState<Record<string, PluginUpdateStatus>>({});
  const [checking, setChecking] = useState(false);

  // Ask the hub to re-fetch every sourced plugin's manifest and report which
  // have a newer published version. Network-bound, so it's explicit rather than
  // on every render; we also fire it once on mount and after an update lands.
  const checkUpdates = useCallback(async () => {
    setChecking(true);
    try {
      const res = await window.electronAPI.checkPluginUpdates?.();
      if (res?.ok && Array.isArray(res.updates)) {
        const byId: Record<string, PluginUpdateStatus> = {};
        for (const u of res.updates as PluginUpdateStatus[]) byId[u.id] = u;
        setUpdates(byId);
      }
    } finally {
      setChecking(false);
    }
  }, []);

  // Check once as soon as there's at least one plugin to check. Keyed on the set
  // of plugin ids so a freshly installed plugin triggers a re-check, but routine
  // status churn (sidecar state) doesn't.
  const pluginIdsKey = plugins.map((p) => p.id).join(',');
  useEffect(() => {
    if (plugins.length > 0) void checkUpdates();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pluginIdsKey, checkUpdates]);

  const updateCount = Object.values(updates).filter((u) => u.hasUpdate).length;
  const togglePerms = (id: string) =>
    setPermsOpen((prev) => {
      const next = new Set(prev);
      next.has(id) ? next.delete(id) : next.add(id);
      return next;
    });

  const remove = async (id: string) => {
    if (!window.confirm(`Remove plugin "${id}"? This stops its server and deletes it.`)) return;
    setBusyId(id);
    try {
      await window.electronAPI.removePlugin?.(id);
    } finally {
      setBusyId(null);
    }
    // usePlugins refetches on the plugin.unloaded event.
  };

  // Reinstall from the recorded install source (download → build → reload). This
  // is the same operation whether the user is taking an offered update or just
  // forcing a fresh pull — installing the source always lands the latest.
  const update = async (id: string, source: string) => {
    setBusyId(id);
    try {
      const res = await window.electronAPI.installPlugin?.(source);
      if (res && !res.ok) window.alert(`Update failed: ${res.error ?? 'unknown error'}`);
    } finally {
      setBusyId(null);
    }
    // usePlugins refetches on the plugin.loaded event; re-check so a consumed
    // update clears its badge (installed version now matches latest).
    void checkUpdates();
  };

  // Toggle the plugin's disabled marker; the hub starts/stops the sidecar.
  const toggle = async (id: string, currentlyDisabled: boolean) => {
    setBusyId(id);
    try {
      const res = await window.electronAPI.setPluginEnabled?.(id, currentlyDisabled);
      if (res && !res.ok)
        window.alert(
          `Failed to ${currentlyDisabled ? 'enable' : 'disable'}: ${res.error ?? 'unknown error'}`,
        );
    } finally {
      setBusyId(null);
    }
  };

  return (
    <div
      style={{
        height: '100%',
        overflow: 'auto',
        background: 'var(--wks-bg-base)',
        color: 'var(--wks-text-primary)',
      }}
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 10,
          padding: '14px 16px',
          borderBottom: '1px solid var(--wks-border-subtle)',
        }}
      >
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 7,
            fontSize: '0.95rem',
            fontWeight: 600,
          }}
        >
          <Blocks size={17} strokeWidth={1.75} /> Plugins
        </div>
        <div style={{ fontSize: '0.68rem', color: 'var(--wks-text-faint)' }}>
          {plugins.length} installed
        </div>
        {updateCount > 0 && (
          <div
            style={{
              fontSize: '0.66rem',
              fontWeight: 600,
              color: 'var(--wks-accent)',
              padding: '2px 8px',
              borderRadius: 'var(--wks-radius-sm)',
              border: '1px solid var(--wks-accent)',
            }}
          >
            {updateCount} update{updateCount === 1 ? '' : 's'} available
          </div>
        )}
        <button
          onClick={() => void checkUpdates()}
          disabled={checking}
          title="Re-check every plugin's install source for a newer version"
          style={{
            display: 'inline-flex',
            alignItems: 'center',
            gap: 5,
            fontSize: '0.72rem',
            fontFamily: 'inherit',
            cursor: checking ? 'default' : 'pointer',
            background: 'transparent',
            color: 'var(--wks-text-secondary)',
            border: '1px solid var(--wks-border-input)',
            borderRadius: 'var(--wks-radius-sm)',
            padding: '5px 10px',
            opacity: checking ? 0.6 : 1,
          }}
        >
          <RefreshCw
            size={12}
            strokeWidth={2}
            style={checking ? { animation: 'wks-spin 1s linear infinite' } : undefined}
          />
          {checking ? 'Checking…' : 'Check for updates'}
        </button>
        <button
          onClick={() => setShowExamples(true)}
          style={{
            marginLeft: 'auto',
            fontSize: '0.72rem',
            fontFamily: 'inherit',
            cursor: 'pointer',
            background: 'transparent',
            color: 'var(--wks-text-secondary)',
            border: '1px solid var(--wks-border-input)',
            borderRadius: 'var(--wks-radius-sm)',
            padding: '5px 12px',
            fontWeight: 600,
          }}
        >
          Browse examples…
        </button>
        <button
          onClick={() => setShowCatalog(true)}
          style={{
            fontSize: '0.72rem',
            fontFamily: 'inherit',
            cursor: 'pointer',
            background: 'transparent',
            color: 'var(--wks-text-secondary)',
            border: '1px solid var(--wks-border-input)',
            borderRadius: 'var(--wks-radius-sm)',
            padding: '5px 12px',
            fontWeight: 600,
          }}
        >
          Browse catalog…
        </button>
        <button
          onClick={() => setShowInstall(true)}
          style={{
            fontSize: '0.72rem',
            fontFamily: 'inherit',
            cursor: 'pointer',
            background: 'var(--wks-accent)',
            color: 'var(--wks-text-on-accent)',
            border: 'none',
            borderRadius: 'var(--wks-radius-sm)',
            padding: '5px 12px',
            fontWeight: 600,
          }}
        >
          ＋ Install from GitHub…
        </button>
      </div>

      {plugins.length === 0 && (
        <div
          style={{
            padding: 30,
            textAlign: 'center',
            color: 'var(--wks-text-faint)',
            fontSize: '0.8rem',
          }}
        >
          No plugins installed. Browse the bundled examples, or install one from a GitHub repo to
          add panes, hotkeys, and dashboards.
        </div>
      )}

      <div style={{ padding: 12, display: 'flex', flexDirection: 'column', gap: 10 }}>
        {plugins.map((p) => {
          const hasServer = !!p.server;
          const sc = sidecar[p.id];
          const state = p.disabled
            ? 'disabled'
            : hasServer
              ? (sc?.state ?? 'starting')
              : 'no server';
          const busy = busyId === p.id;
          const req = pluginRequirement(p);
          const crashErr = state === 'crashed' ? sc?.err : undefined;
          const up = updates[p.id];
          return (
            <div
              key={p.id}
              style={{
                border: '1px solid var(--wks-border-subtle)',
                borderRadius: 'var(--wks-radius-md)',
                padding: 12,
                background: 'var(--wks-bg-surface)',
                opacity: p.disabled ? 0.6 : 1,
              }}
            >
              <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                {hasServer && (
                  <span
                    style={{
                      width: 9,
                      height: 9,
                      borderRadius: '50%',
                      background: stateColor(state),
                      flexShrink: 0,
                    }}
                  />
                )}
                <span style={{ fontSize: '0.85rem', fontWeight: 600 }}>{p.name || p.id}</span>
                <span style={{ fontSize: '0.66rem', color: 'var(--wks-text-faint)' }}>{p.id}</span>
                {p.version && (
                  <span
                    style={{ fontSize: '0.64rem', color: 'var(--wks-text-faint)' }}
                    title="Installed version"
                  >
                    v{p.version}
                  </span>
                )}
                <span
                  style={{ fontSize: '0.66rem', color: 'var(--wks-text-muted)', marginLeft: 6 }}
                >
                  {state}
                </span>
                <div style={{ marginLeft: 'auto', display: 'flex', gap: 6 }}>
                  {/* One button, two meanings: a genuine update (newer version at
                      the source) gets the accent treatment; otherwise it's an
                      honest "Reinstall" — the same reinstall-from-source action,
                      no longer masquerading as a permanent "update available". */}
                  {p.source &&
                    (up?.hasUpdate ? (
                      <button
                        onClick={() => update(p.id, p.source!)}
                        disabled={busy}
                        title={
                          up.latest
                            ? `Update ${p.version ? `v${p.version} → ` : ''}v${up.latest}`
                            : `Update from ${p.source}`
                        }
                        style={updateBtn(busy)}
                      >
                        {busy ? 'Updating…' : up.latest ? `Update → v${up.latest}` : 'Update'}
                      </button>
                    ) : (
                      <button
                        onClick={() => update(p.id, p.source!)}
                        disabled={busy}
                        title={`Reinstall from ${p.source}`}
                        style={actionBtn(busy)}
                      >
                        {busy ? 'Reinstalling…' : 'Reinstall'}
                      </button>
                    ))}
                  <button
                    onClick={() => toggle(p.id, !!p.disabled)}
                    disabled={busy}
                    style={actionBtn(busy)}
                  >
                    {p.disabled ? 'Enable' : 'Disable'}
                  </button>
                  <button
                    onClick={() => remove(p.id)}
                    disabled={busy}
                    style={actionBtn(busy, true)}
                  >
                    {busy ? 'Removing…' : 'Remove'}
                  </button>
                </div>
              </div>
              <div
                style={{
                  marginTop: 8,
                  display: 'flex',
                  gap: 16,
                  flexWrap: 'wrap',
                  fontSize: '0.66rem',
                  color: 'var(--wks-text-muted)',
                }}
              >
                <span>
                  {p.panes?.length ?? 0} pane{(p.panes?.length ?? 0) === 1 ? '' : 's'}
                  {p.panes?.length ? ': ' + p.panes.map((x) => x.title).join(', ') : ''}
                </span>
                {!!p.hotkeys?.length && (
                  <span>hotkeys: {p.hotkeys.map((h) => h.default).join(', ')}</span>
                )}
                {!!p.server?.port && <span>:{p.server.port}</span>}
                {hasServer && req.warn && (
                  <span style={{ color: 'var(--wks-warning)' }}>{req.label}</span>
                )}
                <button
                  onClick={() => togglePerms(p.id)}
                  style={{
                    background: 'none',
                    border: 'none',
                    padding: 0,
                    cursor: 'pointer',
                    fontSize: '0.66rem',
                    fontFamily: 'inherit',
                    color: 'var(--wks-accent)',
                    display: 'inline-flex',
                    alignItems: 'center',
                    gap: 3,
                  }}
                >
                  {hasSensitivePermission(p) && (
                    <AlertTriangle
                      size={10}
                      strokeWidth={2}
                      style={{ color: 'var(--wks-warning)' }}
                    />
                  )}
                  {permsOpen.has(p.id) ? 'Hide permissions' : 'Permissions'}
                </button>
              </div>
              {permsOpen.has(p.id) && (
                <div
                  style={{
                    marginTop: 8,
                    padding: '8px 10px',
                    borderRadius: 'var(--wks-radius-sm)',
                    background: 'var(--wks-bg-input)',
                    border: '1px solid var(--wks-border-subtle)',
                  }}
                >
                  <PluginPermissions manifest={p} compact />
                </div>
              )}
              {crashErr && (
                <div
                  style={{
                    marginTop: 8,
                    padding: '6px 8px',
                    borderRadius: 'var(--wks-radius-sm)',
                    background: 'var(--wks-bg-input)',
                    border: '1px solid var(--wks-border-subtle)',
                    fontSize: '0.64rem',
                    lineHeight: 1.5,
                    color: 'var(--wks-error)',
                    display: 'flex',
                    gap: 5,
                    alignItems: 'flex-start',
                    wordBreak: 'break-word',
                  }}
                >
                  <AlertTriangle
                    size={11}
                    strokeWidth={2}
                    style={{ flexShrink: 0, marginTop: 1 }}
                  />
                  <span>{crashErr}</span>
                </div>
              )}
            </div>
          );
        })}
      </div>

      {showInstall && <PluginInstallDialog onClose={() => setShowInstall(false)} />}
      {showCatalog && (
        <PluginCatalogDialog
          installedIds={plugins.map((p) => p.id)}
          onClose={() => setShowCatalog(false)}
        />
      )}
      {showExamples && (
        <ExamplesGalleryDialog
          installedIds={plugins.map((p) => p.id)}
          onClose={() => setShowExamples(false)}
        />
      )}
    </div>
  );
};

export default PluginsManagerPane;
