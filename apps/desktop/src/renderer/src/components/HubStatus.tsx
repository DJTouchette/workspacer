import React, { useEffect, useState } from 'react';
import { Smartphone } from 'lucide-react';

interface HubEvent {
  id: string;
  type: string;
  source: string;
  time: string;
  data?: unknown;
}

/**
 * Small ambient indicator that the hub event bus is live. Self-subscribes to
 * the IPC-forwarded hub stream — no prop threading. Sits at the bottom of the
 * sidebar. Proof that claudemon → hub → main → renderer round-trips.
 */
const HubStatus: React.FC<{ onOpenRemote?: () => void; compact?: boolean }> = ({ onOpenRemote, compact }) => {
  const [connected, setConnected] = useState(false);
  const [count, setCount] = useState(0);
  const [last, setLast] = useState<HubEvent | null>(null);

  useEffect(() => {
    // Sync the current state on mount — the one-shot `connected:true` push may
    // have fired before this component subscribed.
    window.electronAPI.getHubStatus?.().then((s) => setConnected(s.connected)).catch(() => {});
    const offStatus = window.electronAPI.onHubStatus?.((s) => setConnected(s.connected));
    const offEvent = window.electronAPI.onHubEvent?.((ev) => {
      setCount((n) => n + 1);
      setLast(ev);
    });
    return () => { offStatus?.(); offEvent?.(); };
  }, []);

  const color = connected ? 'var(--wks-success, #3fb950)' : 'var(--wks-text-faint, #666)';
  const title = connected
    ? `hub connected · ${count} events${last ? `\nlast: ${last.type} (${last.source})` : ''}`
    : 'hub disconnected';

  // Rail mode: just the status dot, centered — the full readout has no room.
  if (compact) {
    return (
      <div title={title} style={{
        flexShrink: 0, boxSizing: 'border-box',
        display: 'flex', alignItems: 'center', justifyContent: 'center',
        width: '100%', padding: '8px 0 10px',
        borderTop: '1px solid var(--wks-border-subtle)', background: 'var(--wks-bg-input)',
        userSelect: 'none',
      }}>
        <span style={{
          width: 8, height: 8, borderRadius: '50%', flexShrink: 0,
          backgroundColor: color, boxShadow: connected ? `0 0 6px ${color}` : 'none',
        }} />
      </div>
    );
  }

  return (
    <div
      title={title}
      style={{
        flexShrink: 0,
        boxSizing: 'border-box',
        display: 'flex', alignItems: 'center', gap: 6,
        padding: '8px 12px 10px',
        fontSize: '0.6rem',
        color: 'var(--wks-text-faint)',
        borderTop: '1px solid var(--wks-border-subtle)',
        background: 'var(--wks-bg-input)',
        userSelect: 'none',
      }}
    >
      <span style={{
        width: 6, height: 6, borderRadius: '50%', flexShrink: 0,
        backgroundColor: color,
        boxShadow: connected ? `0 0 5px ${color}` : 'none',
      }} />
      <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
        {connected ? `hub · ${count}` : 'hub offline'}
      </span>
      {onOpenRemote && (
        <button
          onClick={onOpenRemote}
          title="Remote control — drive agents from your phone"
          style={{
            marginLeft: 'auto', flexShrink: 0,
            display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
            background: 'transparent', border: 'none', padding: 2, cursor: 'pointer',
            color: 'var(--wks-text-faint)', borderRadius: 4,
          }}
          onMouseEnter={(e) => { (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-primary)'; }}
          onMouseLeave={(e) => { (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-faint)'; }}
        >
          <Smartphone size={12} />
        </button>
      )}
    </div>
  );
};

export default HubStatus;
