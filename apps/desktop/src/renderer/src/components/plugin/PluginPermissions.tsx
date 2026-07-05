import React from 'react';
import { AlertTriangle } from 'lucide-react';
import type { PluginManifest } from '../../types/plugin';
import { pluginPermissions } from '../../lib/pluginPermissions';

/**
 * The itemized permissions a plugin's manifest declares — the same grants the
 * hub bus enforces (capabilities it can call, events it publishes/receives,
 * calls it answers). Rendered in the install-confirm step (consent) and, in
 * `compact` form, in the plugins manager (audit). Sensitive lines — write
 * access, spawning/steering agents, `command.*`/`*` events — are tinted amber
 * so they stand out from benign ones.
 */
export const PluginPermissions: React.FC<{ manifest: PluginManifest; compact?: boolean }> = ({
  manifest,
  compact,
}) => {
  const groups = pluginPermissions(manifest);
  if (groups.length === 0) {
    return (
      <div style={{ fontSize: compact ? '0.6rem' : '0.66rem', color: 'var(--wks-text-faint)' }}>
        Requests no bus access — panes/hotkeys only.
      </div>
    );
  }
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: compact ? 6 : 9 }}>
      {groups.map((g) => (
        <div key={g.key}>
          <div
            style={{
              fontSize: compact ? '0.56rem' : '0.6rem',
              fontWeight: 700,
              textTransform: 'uppercase',
              letterSpacing: '0.05em',
              color: 'var(--wks-text-faint)',
              marginBottom: 3,
            }}
          >
            {g.title}
          </div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
            {g.lines.map((l, i) => (
              <div
                key={i}
                style={{
                  display: 'flex',
                  alignItems: 'baseline',
                  gap: 6,
                  fontSize: compact ? '0.64rem' : '0.7rem',
                }}
              >
                <span
                  style={{
                    flexShrink: 0,
                    width: 12,
                    display: 'inline-flex',
                    justifyContent: 'center',
                    color:
                      l.severity === 'sensitive'
                        ? 'var(--wks-warning, #e0a000)'
                        : 'var(--wks-text-faint)',
                  }}
                >
                  {l.severity === 'sensitive' ? (
                    <AlertTriangle size={compact ? 10 : 11} strokeWidth={2} />
                  ) : (
                    '•'
                  )}
                </span>
                <span
                  style={{
                    color:
                      l.severity === 'sensitive'
                        ? 'var(--wks-text-primary)'
                        : 'var(--wks-text-secondary)',
                    fontWeight: l.severity === 'sensitive' ? 600 : 400,
                  }}
                >
                  {l.label}
                  {l.detail && (
                    <span style={{ color: 'var(--wks-text-faint)', fontWeight: 400 }}>
                      {' '}
                      — {l.detail}
                    </span>
                  )}
                </span>
              </div>
            ))}
          </div>
        </div>
      ))}
    </div>
  );
};

export default PluginPermissions;
