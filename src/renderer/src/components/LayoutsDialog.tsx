import React, { useEffect, useState } from 'react';
import type { Layout } from '../types/layout';

interface Props {
  /** Number of (non-global) agents that would be captured by "Save current". */
  agentCount: number;
  onSaveCurrent: (name: string) => void;
  onRestore: (layout: Layout) => void;
  onClose: () => void;
}

/**
 * Manage layout templates: save the current arrangement of directories + panes
 * under a name, and restore a saved one (spawns fresh agents for its dirs).
 */
const LayoutsDialog: React.FC<Props> = ({ agentCount, onSaveCurrent, onRestore, onClose }) => {
  const [layouts, setLayouts] = useState<Layout[]>([]);
  const [name, setName] = useState('');

  const reload = () => {
    window.electronAPI.layoutsList().then((l) => setLayouts(Array.isArray(l) ? l : [])).catch(() => {});
  };
  useEffect(() => { reload(); }, []);

  useEffect(() => {
    const handler = (e: KeyboardEvent) => { if (e.key === 'Escape') { e.preventDefault(); onClose(); } };
    window.addEventListener('keydown', handler, true);
    return () => window.removeEventListener('keydown', handler, true);
  }, [onClose]);

  const save = () => {
    const n = name.trim();
    if (!n) return;
    onSaveCurrent(n);
    setName('');
    // give the main process a beat to write, then refresh the list
    setTimeout(reload, 150);
  };

  const del = (id: string) => {
    window.electronAPI.layoutsDelete(id).then(reload).catch(() => {});
  };

  return (
    <div onClick={onClose} style={overlay}>
      <div onClick={(e) => e.stopPropagation()} style={modal}>
        <div style={{ fontSize: '0.95rem', fontWeight: 700, color: 'var(--wks-text-primary)', marginBottom: 4 }}>Layouts</div>
        <div style={{ fontSize: '0.62rem', color: 'var(--wks-text-faint)', marginBottom: 14 }}>
          Reusable directory + pane templates. Restoring spawns fresh agents.
        </div>

        {/* Save current */}
        <div style={{ display: 'flex', gap: 8, marginBottom: 16 }}>
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => { if (e.key === 'Enter') save(); }}
            placeholder={agentCount > 0 ? `Save ${agentCount} agent${agentCount === 1 ? '' : 's'} as…` : 'No agents to save'}
            disabled={agentCount === 0}
            style={input}
            autoFocus
          />
          <button onClick={save} disabled={agentCount === 0 || !name.trim()} style={primaryBtn}>Save current</button>
        </div>

        {/* Saved layouts */}
        <div style={{ fontSize: '0.6rem', color: 'var(--wks-text-faint)', textTransform: 'uppercase', letterSpacing: '0.05em', marginBottom: 6 }}>
          Saved layouts
        </div>
        <div style={{ overflow: 'auto', flex: 1, minHeight: 0 }}>
          {layouts.length === 0 ? (
            <div style={{ fontSize: '0.7rem', color: 'var(--wks-text-faint)', padding: '8px 0' }}>None yet.</div>
          ) : layouts.map((l) => (
            <div key={l.id} style={row}
              onMouseEnter={(e) => { (e.currentTarget as HTMLElement).style.backgroundColor = 'var(--wks-bg-hover)'; }}
              onMouseLeave={(e) => { (e.currentTarget as HTMLElement).style.backgroundColor = 'transparent'; }}>
              <div style={{ flex: 1, minWidth: 0, cursor: 'pointer' }} onClick={() => { onRestore(l); onClose(); }}>
                <div style={{ fontSize: '0.78rem', color: 'var(--wks-text-primary)', fontWeight: 600 }}>{l.name}</div>
                <div style={{ fontSize: '0.6rem', color: 'var(--wks-text-faint)', marginTop: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                  {l.agents.length} {l.agents.length === 1 ? 'dir' : 'dirs'}: {l.agents.map((a) => a.name).join(', ')}
                </div>
              </div>
              <button onClick={() => { onRestore(l); onClose(); }} style={miniBtn} title="Restore — spawn fresh agents">Restore</button>
              <button onClick={() => del(l.id)} style={{ ...miniBtn, color: 'var(--wks-danger, #ff8a8a)' }} title="Delete layout">✕</button>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
};

const overlay: React.CSSProperties = {
  position: 'fixed', inset: 0, zIndex: 2500, backgroundColor: 'rgba(0,0,0,0.4)',
  display: 'flex', alignItems: 'center', justifyContent: 'center',
};
const modal: React.CSSProperties = {
  backgroundColor: 'var(--wks-bg-raised)', border: '1px solid var(--wks-border)', borderRadius: 8,
  padding: '18px 20px', width: 460, maxWidth: '90vw', maxHeight: '70vh', display: 'flex', flexDirection: 'column',
};
const input: React.CSSProperties = {
  flex: 1, height: 30, padding: '0 10px', fontSize: '0.75rem', fontFamily: 'inherit',
  background: 'var(--wks-bg-input, var(--wks-bg-base))', color: 'var(--wks-text-primary)',
  border: '1px solid var(--wks-border-input)', borderRadius: 5, margin: 0,
};
const primaryBtn: React.CSSProperties = {
  padding: '0 14px', height: 30, fontSize: '0.72rem', fontWeight: 600, fontFamily: 'inherit',
  background: 'var(--wks-accent)', color: '#fff', border: '1px solid var(--wks-accent)', borderRadius: 5,
  cursor: 'pointer', margin: 0, whiteSpace: 'nowrap',
};
const row: React.CSSProperties = {
  display: 'flex', alignItems: 'center', gap: 8, padding: '7px 8px', borderRadius: 6,
};
const miniBtn: React.CSSProperties = {
  padding: '3px 9px', fontSize: '0.66rem', fontFamily: 'inherit', background: 'transparent',
  color: 'var(--wks-text-muted)', border: '1px solid var(--wks-border-input)', borderRadius: 5,
  cursor: 'pointer', margin: 0, flexShrink: 0,
};

export default LayoutsDialog;
