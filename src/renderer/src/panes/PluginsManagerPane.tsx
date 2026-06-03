import React, { useEffect, useState } from 'react';
import { usePlugins } from '../hooks/usePlugins';
import PluginInstallDialog from '../components/PluginInstallDialog';
import { Blocks } from '../components/icons';

/** Latest supervisor state per plugin id, from `sidecar.*` bus events. */
function useSidecarStates(): Record<string, string> {
  const [states, setStates] = useState<Record<string, string>>({});
  useEffect(() => {
    const off = window.electronAPI.onHubEvent?.((ev) => {
      if (!ev.type?.startsWith('sidecar.')) return;
      const d = ev.data as { name?: string; state?: string } | undefined;
      if (d?.name && d?.state) setStates((prev) => ({ ...prev, [d.name as string]: d.state as string }));
    });
    return () => off?.();
  }, []);
  return states;
}

function stateColor(s: string | undefined): string {
  switch (s) {
    case 'healthy':
    case 'running': return 'var(--wks-success, #3fb950)';
    case 'unhealthy': return 'var(--wks-warning, #e0a000)';
    case 'crashed': return 'var(--wks-danger, #e05555)';
    case 'stopped': return 'var(--wks-text-faint, #666)';
    default: return 'var(--wks-text-faint, #666)';
  }
}

const PluginsManagerPane: React.FC<{ title?: string }> = () => {
  const { plugins } = usePlugins();
  const sidecar = useSidecarStates();
  const [showInstall, setShowInstall] = useState(false);
  const [busyId, setBusyId] = useState<string | null>(null);

  const remove = async (id: string) => {
    if (!window.confirm(`Remove plugin "${id}"? This stops its server and deletes it.`)) return;
    setBusyId(id);
    try { await window.electronAPI.removePlugin?.(id); } finally { setBusyId(null); }
    // usePlugins refetches on the plugin.unloaded event.
  };

  return (
    <div style={{ height: '100%', overflow: 'auto', background: 'var(--wks-bg-base)', color: 'var(--wks-text-primary)' }}>
      <div style={{
        display: 'flex', alignItems: 'center', gap: 10, padding: '14px 16px',
        borderBottom: '1px solid var(--wks-border-subtle)',
      }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 7, fontSize: '0.95rem', fontWeight: 600 }}>
          <Blocks size={17} strokeWidth={1.75} /> Plugins
        </div>
        <div style={{ fontSize: '0.65rem', color: 'var(--wks-text-faint)' }}>{plugins.length} installed</div>
        <button
          onClick={() => setShowInstall(true)}
          style={{
            marginLeft: 'auto', fontSize: '0.72rem', fontFamily: 'inherit', cursor: 'pointer',
            background: 'var(--wks-accent)', color: 'var(--wks-text-on-accent, #fff)',
            border: 'none', borderRadius: 5, padding: '5px 12px', fontWeight: 600,
          }}
        >＋ Install from GitHub…</button>
      </div>

      {plugins.length === 0 && (
        <div style={{ padding: 30, textAlign: 'center', color: 'var(--wks-text-faint)', fontSize: '0.8rem' }}>
          No plugins installed. Install one from a GitHub repo to add panes, hotkeys, and dashboards.
        </div>
      )}

      <div style={{ padding: 12, display: 'flex', flexDirection: 'column', gap: 10 }}>
        {plugins.map((p) => {
          const state = p.server ? (sidecar[p.id] ?? 'starting') : 'no server';
          const hasServer = !!p.server;
          return (
            <div key={p.id} style={{
              border: '1px solid var(--wks-border-subtle)', borderRadius: 8, padding: 12,
              background: 'var(--wks-bg-surface)',
            }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                {hasServer && (
                  <span style={{ width: 9, height: 9, borderRadius: '50%', background: stateColor(sidecar[p.id]), flexShrink: 0 }} />
                )}
                <span style={{ fontSize: '0.85rem', fontWeight: 600 }}>{p.name || p.id}</span>
                <span style={{ fontSize: '0.62rem', color: 'var(--wks-text-faint)' }}>{p.id}</span>
                <span style={{ fontSize: '0.62rem', color: 'var(--wks-text-muted)', marginLeft: 6 }}>{state}</span>
                <button
                  onClick={() => remove(p.id)}
                  disabled={busyId === p.id}
                  style={{
                    marginLeft: 'auto', fontSize: '0.68rem', fontFamily: 'inherit',
                    cursor: busyId === p.id ? 'default' : 'pointer',
                    background: 'transparent', color: 'var(--wks-danger, #e05555)',
                    border: '1px solid var(--wks-border-input)', borderRadius: 4, padding: '3px 10px',
                  }}
                >{busyId === p.id ? 'Removing…' : 'Remove'}</button>
              </div>
              <div style={{ marginTop: 8, display: 'flex', gap: 16, flexWrap: 'wrap', fontSize: '0.62rem', color: 'var(--wks-text-muted)' }}>
                <span>{(p.panes?.length ?? 0)} pane{(p.panes?.length ?? 0) === 1 ? '' : 's'}{p.panes?.length ? ': ' + p.panes.map((x) => x.title).join(', ') : ''}</span>
                {!!p.hotkeys?.length && <span>hotkeys: {p.hotkeys.map((h) => h.default).join(', ')}</span>}
                {!!p.server?.port && <span>:{p.server.port}</span>}
              </div>
            </div>
          );
        })}
      </div>

      {showInstall && <PluginInstallDialog onClose={() => setShowInstall(false)} />}
    </div>
  );
};

export default PluginsManagerPane;
