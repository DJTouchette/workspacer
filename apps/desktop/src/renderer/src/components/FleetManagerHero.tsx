import { useProviderReadiness } from '../hooks/useProviderReadiness';
import { useAgentRuntimeStatus } from '../hooks/useAgentRuntimeStatus';
import { SmallButton } from './settings/primitives';
import { useConfig } from '../hooks/useConfig';
import { useProviderDetection } from '../hooks/useProviderDetection';
import { providerAvailability } from '../lib/providerAvailability';
/**
 * FleetManagerHero — the Overview's front-and-center entry to the Fleet
 * Manager (FLEET_MANAGER_SPIKE.md direction a): one ask box + preset chips.
 * Dispatches `fleet-manager:ask`; App resolves the fleet root and
 * spawns-or-reuses the manager (useAgentManager.spawnFleetManager). The ask
 * stays here until App settles the launch; binary discovery is not sign-in.
 */
import React, { useState, useRef, useEffect } from 'react';
import { Brain } from './icons';
import { MANAGER_PRESETS } from '../lib/fleetManager';

const FleetManagerHero: React.FC = () => {
  const [ask, setAsk] = useState('');
  const [error, setError] = useState('');
  const inputRef = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (error) inputRef.current?.focus();
  }, [error]);
  const [busy, setBusy] = useState(false);
  const pending = useRef(false);
  const { config } = useConfig();
  const { detection, refresh } = useProviderDetection();
  const provider = config.agents?.managerProvider ?? 'claude';
  const readiness = useProviderReadiness(provider);
  const runtimeStatus = useAgentRuntimeStatus(true);
  const missing = providerAvailability(detection, provider) === 'missing';
  const submit = (text: string) => {
    const trimmed = text.trim();
    if (pending.current || (trimmed && (missing || runtimeStatus.blocked))) return;
    setAsk(text);
    setError('');
    setBusy(true);
    pending.current = true;
    window.dispatchEvent(
      new CustomEvent('fleet-manager:ask', {
        detail: {
          ask: text,
          onSettled: (failure?: string) => {
            pending.current = false;
            setBusy(false);
            if (failure) setError(failure);
            else setAsk('');
          },
        },
      }),
    );
  };
  return (
    <div
      style={{
        marginBottom: 22,
        padding: '14px 16px',
        borderRadius: 'var(--wks-radius-md)',
        border: '1px solid color-mix(in srgb, var(--wks-accent) 30%, var(--wks-border))',
        background: 'color-mix(in srgb, var(--wks-accent) 6%, transparent)',
      }}
    >
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 8 }}>
        <Brain size={16} strokeWidth={1.75} style={{ color: 'var(--wks-accent-text)' }} />
        <span style={{ fontWeight: 700, fontSize: '0.85rem' }}>Fleet Manager</span>
        <span style={{ fontSize: '0.72rem', color: 'var(--wks-text-muted)' }}>
          tell it what to get done across your projects — it dispatches agents and reports back
        </span>
      </div>
      <input
        ref={inputRef}
        aria-label="Ask the Fleet Manager"
        disabled={busy}
        value={ask}
        onChange={(e) => setAsk(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter') submit(ask);
        }}
        placeholder="e.g. “kick off the profile bugfix in preheat, and have someone update the docs here”"
        style={{
          width: '100%',
          boxSizing: 'border-box',
          height: 34,
          padding: '0 10px',
          fontSize: '0.8rem',
          borderRadius: 'var(--wks-radius-sm)',
          border: '1px solid var(--wks-border-input)',
          background: 'var(--wks-bg-input)',
          color: 'var(--wks-text-primary)',
          outline: 'none',
        }}
      />
      <div id="fleet-provider-status" role="status" style={{ fontSize: '0.72rem', marginTop: 8 }}>
        {missing
          ? `${provider} is not installed. Choose an installed Fleet Manager provider or set its binary override in Settings.`
          : providerAvailability(detection, provider) === 'installed'
            ? `${provider} CLI found.`
            : `${provider} availability is unknown; you can try starting it.`}
        <span> {readiness.detail}</span>
        <SmallButton
          onClick={() => {
            refresh();
            void readiness.refresh();
          }}
          label="Check again"
        />
      </div>
      <div
        id="fleet-runtime-status"
        aria-live="polite"
        style={{ fontSize: '0.72rem', marginTop: 8 }}
      >
        {runtimeStatus.detail}
        <SmallButton onClick={() => void runtimeStatus.refresh()} label="Check runtime again" />
      </div>
      {error && (
        <div role="alert" style={{ color: 'var(--wks-error)', marginTop: 8 }}>
          {error}
        </div>
      )}
      <button
        style={{
          marginTop: 8,
          padding: '6px 12px',
          borderRadius: 'var(--wks-radius-sm)',
          border: '1px solid var(--wks-border-input)',
          background: 'var(--wks-accent-bg)',
          color: 'var(--wks-accent-text)',
          fontFamily: 'inherit',
          fontSize: '0.8rem',
          cursor:
            busy || (!!ask.trim() && (missing || runtimeStatus.blocked)) ? 'default' : 'pointer',
          opacity: busy || (!!ask.trim() && (missing || runtimeStatus.blocked)) ? 0.5 : 1,
        }}
        onClick={() => submit(ask)}
        disabled={busy || (!!ask.trim() && (missing || runtimeStatus.blocked))}
        aria-describedby="fleet-provider-status fleet-runtime-status"
      >
        {busy ? 'Starting…' : ask.trim() ? 'Ask Fleet Manager' : 'Open Fleet Manager'}
      </button>
      <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap', marginTop: 8 }}>
        {MANAGER_PRESETS.map((p) => (
          <button
            key={p.id}
            disabled={busy || missing || runtimeStatus.blocked}
            onClick={() => submit(p.prompt)}
            style={{
              fontSize: '0.7rem',
              padding: '3px 10px',
              borderRadius: 999,
              border: '1px solid var(--wks-border-input)',
              background: 'transparent',
              color: 'var(--wks-text-secondary)',
              cursor: 'pointer',
            }}
          >
            {p.label}
          </button>
        ))}
      </div>
    </div>
  );
};

export default FleetManagerHero;
