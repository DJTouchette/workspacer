import React from 'react';
import { Config } from '../../hooks/useConfig';
import { resolveUiMode, type UiMode } from '../../lib/uiMode';
import { Section, Row, ModeButton } from './primitives';

interface LayoutSectionProps {
  config: Config;
  save: (partial: Partial<Config>) => Promise<Config>;
}

const LayoutSection: React.FC<LayoutSectionProps> = ({ config, save }) => {
  const uiMode = resolveUiMode(config.ui?.mode);
  const setUiMode = (mode: UiMode) => save({ ui: { ...config.ui, mode } });
  return (
    <Section title="Layout">
      <Row label="UI mode">
        <div style={{ display: 'flex', gap: '6px' }}>
          <ModeButton label="Full" active={uiMode === 'fleet'} onClick={() => setUiMode('fleet')} />
          <ModeButton
            label="Focus"
            active={uiMode === 'focus'}
            onClick={() => setUiMode('focus')}
          />
        </div>
      </Row>
      <Row label="Peek">
        <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
          <input
            type="range"
            min={0}
            max={200}
            step={10}
            value={config.panes?.peek ?? 0}
            onChange={(e) => save({ panes: { ...config.panes, peek: parseInt(e.target.value) } })}
            style={{ width: '120px', accentColor: 'var(--wks-accent)' }}
          />
          <span
            style={{
              fontSize: '0.7rem',
              fontFamily: 'var(--wks-font-mono)',
              color: 'var(--wks-text-tertiary)',
              minWidth: '32px',
            }}
          >
            {config.panes?.peek ?? 0}px
          </span>
        </div>
      </Row>
      <Row label="Gap">
        <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
          <input
            type="range"
            min={0}
            max={40}
            step={2}
            value={config.panes?.gap ?? 0}
            onChange={(e) => save({ panes: { ...config.panes, gap: parseInt(e.target.value) } })}
            style={{ width: '120px', accentColor: 'var(--wks-accent)' }}
          />
          <span
            style={{
              fontSize: '0.7rem',
              fontFamily: 'var(--wks-font-mono)',
              color: 'var(--wks-text-tertiary)',
              minWidth: '32px',
            }}
          >
            {config.panes?.gap ?? 0}px
          </span>
        </div>
      </Row>
    </Section>
  );
};

export default LayoutSection;
