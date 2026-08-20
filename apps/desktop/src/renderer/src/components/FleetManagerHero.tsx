/**
 * FleetManagerHero — the Overview's front-and-center entry to the Fleet
 * Manager (FLEET_MANAGER_SPIKE.md direction a): one ask box + preset chips.
 * Dispatches `fleet-manager:ask`; App resolves the fleet root and
 * spawns-or-reuses the manager (useAgentManager.spawnFleetManager). Kept
 * dumb on purpose — no config, no agent state — so it renders anywhere.
 */
import React, { useState } from 'react';
import { Brain } from './icons';
import { MANAGER_PRESETS } from '../lib/fleetManager';

const FleetManagerHero: React.FC = () => {
  const [ask, setAsk] = useState('');
  const submit = (text: string) => {
    const trimmed = text.trim();
    if (!trimmed) return;
    window.dispatchEvent(new CustomEvent('fleet-manager:ask', { detail: { ask: trimmed } }));
    setAsk('');
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
      <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap', marginTop: 8 }}>
        {MANAGER_PRESETS.map((p) => (
          <button
            key={p.id}
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
