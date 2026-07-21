import React, { useCallback, useEffect, useState } from 'react';
import { Section, SmallButton } from './primitives';
import type { ExternalToolStatus } from '../../types/electron';

/**
 * Settings → System Tools: the external-tool registry (main/toolCheck.ts) as a
 * health board — every system binary features shell out to, a green/red dot,
 * the resolved path or the install hint, and what stops working without it.
 * "Re-check" re-scans PATH after the user installs something.
 */
const ToolsSection: React.FC = () => {
  const [tools, setTools] = useState<ExternalToolStatus[] | null>(null);
  const [checking, setChecking] = useState(false);

  const load = useCallback(async (force = false) => {
    if (force) setChecking(true);
    try {
      const list = await window.electronAPI.toolsStatus?.(force);
      setTools(list ?? []);
    } catch {
      setTools([]);
    } finally {
      setChecking(false);
    }
  }, []);

  useEffect(() => {
    // Fresh scan on open — the cached answer may predate an install.
    void load(true);
  }, [load]);

  return (
    <Section title="System Tools">
      <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-muted)', marginBottom: 4 }}>
        External programs Workspacer features depend on. Missing ones only disable the features
        listed next to them — everything else keeps working.
      </div>

      {tools === null && (
        <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-faint)', padding: '12px 0' }}>
          Checking…
        </div>
      )}

      {tools?.map((t) => (
        <div
          key={t.id}
          style={{
            display: 'flex',
            alignItems: 'flex-start',
            gap: 12,
            padding: '13px 0',
            borderTop: '1px solid var(--wks-border-subtle)',
          }}
        >
          <span
            title={t.available ? 'Installed' : 'Not found on PATH'}
            style={{
              width: 9,
              height: 9,
              borderRadius: 'var(--wks-radius-pill)',
              marginTop: 4,
              flexShrink: 0,
              background: t.available ? 'var(--wks-success)' : 'var(--wks-error)',
            }}
          />
          <div style={{ flex: 1, minWidth: 0 }}>
            <div
              style={{
                display: 'flex',
                alignItems: 'baseline',
                gap: 8,
                fontSize: '0.85rem',
                fontWeight: 500,
                color: 'var(--wks-text-primary)',
              }}
            >
              {t.label}
              <span
                style={{
                  fontSize: '0.7rem',
                  fontFamily: 'var(--wks-font-mono)',
                  color: 'var(--wks-text-faint)',
                }}
              >
                {t.bin}
              </span>
            </div>
            <div style={{ fontSize: '0.7rem', color: 'var(--wks-text-muted)', marginTop: 3 }}>
              {t.features.join(' · ')}
            </div>
            <div
              style={{
                fontSize: '0.7rem',
                fontFamily: 'var(--wks-font-mono)',
                color: t.available ? 'var(--wks-text-faint)' : 'var(--wks-warning)',
                marginTop: 3,
                overflow: 'hidden',
                textOverflow: 'ellipsis',
                whiteSpace: 'nowrap',
              }}
              title={t.available ? t.path : t.install}
            >
              {t.available ? t.path : t.install}
            </div>
          </div>
        </div>
      ))}

      <div style={{ paddingTop: 14, borderTop: '1px solid var(--wks-border-subtle)' }}>
        <SmallButton
          label={checking ? 'Re-checking…' : 'Re-check'}
          onClick={() => void load(true)}
        />
      </div>
    </Section>
  );
};

export default ToolsSection;
