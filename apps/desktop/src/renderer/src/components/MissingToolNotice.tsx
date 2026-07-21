import React, { useState } from 'react';
import { Wrench } from 'lucide-react';

import type { ExternalToolStatus } from '../types/electron';

/**
 * Friendly full-surface notice for a feature whose external tool is missing
 * (registry: main/services/toolCheck.ts) — shown instead of the feature's
 * content, replacing raw ENOENT errors. Lists what needs the tool, gives the
 * platform's install hint, and offers "Check again" (re-scans PATH) so the
 * user can install and continue without restarting.
 */
export const MissingToolNotice: React.FC<{
  tool: ExternalToolStatus;
  /** The feature the user just opened (leads the headline). */
  feature: string;
  /** Called when a re-check finds the tool — the caller re-renders its content. */
  onAvailable: () => void;
}> = ({ tool, feature, onAvailable }) => {
  const [checking, setChecking] = useState(false);
  const [stillMissing, setStillMissing] = useState(false);

  const recheck = async () => {
    setChecking(true);
    setStillMissing(false);
    try {
      const list = await window.electronAPI.toolsStatus?.(true);
      const entry = list?.find((t) => t.id === tool.id);
      if (entry?.available) onAvailable();
      else setStillMissing(true);
    } catch {
      setStillMissing(true);
    } finally {
      setChecking(false);
    }
  };

  return (
    <div
      style={{
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        justifyContent: 'center',
        height: '100%',
        gap: 10,
        padding: 24,
        textAlign: 'center',
        animation: 'wks-fade-in 0.25s ease-out',
      }}
    >
      <div
        style={{
          width: 46,
          height: 46,
          borderRadius: '50%',
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          color: 'var(--wks-warning)',
          border: '1px solid color-mix(in srgb, var(--wks-warning) 45%, transparent)',
          background: 'color-mix(in srgb, var(--wks-warning) 10%, transparent)',
        }}
      >
        <Wrench size={19} strokeWidth={2} />
      </div>
      <div style={{ fontSize: '0.9rem', fontWeight: 650, color: 'var(--wks-text-primary)' }}>
        {feature} needs {tool.label}
      </div>
      <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-muted)', maxWidth: 420 }}>
        <code
          style={{
            fontFamily: 'var(--wks-font-mono)',
            color: 'var(--wks-text-secondary)',
          }}
        >
          {tool.bin}
        </code>{' '}
        isn&apos;t on your PATH. It also powers: {tool.features.join(' · ')}.
      </div>
      <div
        style={{
          fontSize: '0.72rem',
          fontFamily: 'var(--wks-font-mono)',
          color: 'var(--wks-text-secondary)',
          padding: '7px 12px',
          borderRadius: 'var(--wks-radius-sm)',
          border: '1px solid var(--wks-border-subtle)',
          background: 'var(--wks-bg-raised)',
          maxWidth: 460,
        }}
      >
        {tool.install}
      </div>
      <button
        onClick={() => void recheck()}
        disabled={checking}
        style={{
          marginTop: 6,
          fontSize: '0.72rem',
          fontFamily: 'inherit',
          fontWeight: 600,
          cursor: checking ? 'default' : 'pointer',
          background: 'var(--wks-accent)',
          color: 'var(--wks-text-on-accent)',
          border: 'none',
          borderRadius: 7,
          padding: '7px 16px',
          opacity: checking ? 0.6 : 1,
        }}
      >
        {checking ? 'Checking…' : 'Check again'}
      </button>
      {stillMissing && (
        <div style={{ fontSize: '0.69rem', color: 'var(--wks-text-faint)' }}>
          Still not found — new PATH entries may need an app restart to be seen.
        </div>
      )}
    </div>
  );
};

export default MissingToolNotice;
