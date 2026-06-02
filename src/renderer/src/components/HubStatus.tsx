import React, { useEffect, useState } from 'react';

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
const HubStatus: React.FC = () => {
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
    </div>
  );
};

export default HubStatus;
