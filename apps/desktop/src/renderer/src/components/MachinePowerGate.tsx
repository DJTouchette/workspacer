import React, { useEffect, useState, useSyncExternalStore } from 'react';
import { Power } from 'lucide-react';
import {
  machinePowerSnapshot,
  subscribeMachinePower,
  stopMachine,
  wakeMachine,
  refreshMachinePower,
} from '../backend/machinePower';
import { SmallButton } from './settings/primitives';
import RemoteShareDialog from './RemoteShareDialog';

/** Unmount the app on deliberate stop so its pollers and plugin frames cannot
 * wake the machine. This gate itself makes no background network requests. */
export default function MachinePowerGate({
  children,
}: {
  children: React.ReactNode;
}): React.JSX.Element {
  const power = useSyncExternalStore(subscribeMachinePower, machinePowerSnapshot);
  const [confirm, setConfirm] = useState(false);
  const [busy, setBusy] = useState(false);
  const [connectionSettings, setConnectionSettings] = useState(false);
  const [idleOpen, setIdleOpen] = useState(false);
  useEffect(() => {
    if (!idleOpen || !power.connected) return;
    void refreshMachinePower();
    const timer = setInterval(() => {
      void refreshMachinePower();
    }, 30000);
    return () => clearInterval(timer);
  }, [idleOpen, power.connected]);
  const controls = (
    <>
      {power.error && <p role="alert">{power.error}</p>}
      {power.idleMode && power.idleMode !== 'off' && (
        <details open={idleOpen} onToggle={(e) => setIdleOpen(e.currentTarget.open)}>
          <summary style={{ cursor: 'pointer' }}>
            Idle: {power.idleMode === 'observe' ? 'observing' : 'auto-stop'}
          </summary>
          <p>
            {power.idleMode === 'observe'
              ? 'Observation only; automatic shutdown is disabled.'
              : 'Automatic shutdown is enabled.'}
          </p>
          <p>
            {Math.floor((power.idle?.calmSeconds ?? 0) / 60)} /{' '}
            {Math.ceil((power.idle?.dwellSeconds ?? 0) / 60)} minutes quiet
          </p>
          {power.idle?.quiescent ? (
            <p>Ready to stop.</p>
          ) : (
            <ul>
              {power.idle?.blockers?.map((b, i) => (
                <li key={i}>{b.detail}</li>
              ))}
            </ul>
          )}
        </details>
      )}
      {power.canStop &&
        (confirm ? (
          <>
            <span>
              Stop {power.label}? This ends running work and disconnects everyone. Storage charges
              continue.
            </span>
            <SmallButton
              label="Stop machine"
              danger
              disabled={busy}
              onClick={() => {
                setBusy(true);
                void stopMachine().finally(() => {
                  setBusy(false);
                  setConfirm(false);
                });
              }}
            />
            <SmallButton label="Cancel" onClick={() => setConfirm(false)} />
          </>
        ) : (
          <SmallButton
            label={
              <>
                <Power size={12} /> Stop server
              </>
            }
            disabled={!power.connected}
            onClick={() => setConfirm(true)}
          />
        ))}
    </>
  );
  if (power.paused)
    return (
      <div
        style={{
          minHeight: '100vh',
          display: 'grid',
          placeContent: 'center',
          padding: 20,
          background: 'var(--wks-bg-base)',
          color: 'var(--wks-text-primary)',
          fontFamily: 'var(--wks-font-sans)',
        }}
      >
        <h1 style={{ fontSize: '1.05rem' }}>Server disconnected</h1>
        <p>Stop was requested. Automatic reconnect is paused.</p>
        <p>Allow the machine to finish shutting down before waking it.</p>
        <SmallButton
          label={power.waking ? 'Waking…' : 'Wake server'}
          disabled={power.waking}
          onClick={() => {
            void wakeMachine();
          }}
        />
        {power.error && <p role="alert">{power.error}</p>}
        {window.electronAPI?.setRemoteServer && (
          <SmallButton label="Connection settings" onClick={() => setConnectionSettings(true)} />
        )}
        {connectionSettings && <RemoteShareDialog onClose={() => setConnectionSettings(false)} />}
        <p style={{ color: 'var(--wks-text-secondary)', fontSize: '0.72rem' }}>
          Wake reconnects to the server. Starting may take a minute.
        </p>
      </div>
    );
  return (
    <>
      {(power.canStop || (power.idleMode && power.idleMode !== 'off')) && (
        <div
          style={{
            position: 'fixed',
            bottom: 12,
            right: 12,
            zIndex: 10000,
            display: 'flex',
            alignItems: 'center',
            gap: 8,
            padding: '8px 12px',
            maxWidth: 'min(600px, calc(100vw - 24px))',
            flexWrap: 'wrap',
            background: 'var(--wks-bg-elevated)',
            color: 'var(--wks-text-primary)',
            borderRadius: 'var(--wks-radius-md)',
            fontSize: '0.72rem',
          }}
        >
          {controls}
        </div>
      )}
      {children}
    </>
  );
}
