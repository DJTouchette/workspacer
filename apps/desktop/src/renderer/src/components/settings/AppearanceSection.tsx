import React, { useCallback, useRef, useState } from 'react';
import { Config } from '../../hooks/useConfig';
import { themes, darkTheme, toHex } from '../../themes';
import { Section, Row, ModeButton, SearchableSelect } from './primitives';

interface AppearanceSectionProps {
  config: Config;
  save: (partial: Partial<Config>) => Promise<Config>;
}

/** "tokyo-night" → "Tokyo Night", "ayu-dark" → "Ayu Dark". */
function prettyThemeLabel(id: string): string {
  return id.split('-').map((w) => w.charAt(0).toUpperCase() + w.slice(1)).join(' ');
}

const AppearanceSection: React.FC<AppearanceSectionProps> = ({ config, save }) => {
  const activeTheme = themes[config.ui.theme] ?? darkTheme;
  const borderHex = toHex(config.ui.borderColor || activeTheme.borderActive || activeTheme.accent);
  const [saved, setSaved] = useState(false);
  const savedTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const themeOptions = React.useMemo(
    () => Object.entries(themes).map(([id, t]) => ({ value: id, label: prettyThemeLabel(id), swatch: t.accent })),
    [],
  );

  const saveWithFeedback = useCallback(async (partial: Partial<Config>) => {
    await save(partial);
    setSaved(true);
    if (savedTimerRef.current) clearTimeout(savedTimerRef.current);
    savedTimerRef.current = setTimeout(() => setSaved(false), 1500);
  }, [save]);

  return (
    <Section title="Appearance">
      {/* Saved feedback chip */}
      {saved && (
        <div style={{
          display: 'inline-flex', alignItems: 'center', gap: '4px',
          fontSize: '0.6rem', color: 'var(--wks-accent)',
          background: 'var(--wks-bg-selected)', border: '1px solid var(--wks-accent)',
          borderRadius: '10px', padding: '1px 8px', marginBottom: '6px',
          animation: 'wks-fade-in 0.15s ease',
        }}>
          Saved ✓
        </div>
      )}
      <Row label="Theme">
        <SearchableSelect
          value={config.ui.theme}
          options={themeOptions}
          /* Switching theme re-adopts that theme's own corner style and
           * border color (overrides cleared). */
          onChange={(themeName) => saveWithFeedback({ ui: { ...config.ui, theme: themeName, cornerStyle: '', borderColor: '' } })}
        />
      </Row>
      <Row label="Corners">
        <div style={{ display: 'flex', gap: '4px' }}>
          <ModeButton
            label="Theme"
            active={!config.ui.cornerStyle}
            onClick={() => saveWithFeedback({ ui: { ...config.ui, cornerStyle: '' } })}
          />
          <ModeButton
            label="Rounded"
            active={config.ui.cornerStyle === 'rounded'}
            onClick={() => saveWithFeedback({ ui: { ...config.ui, cornerStyle: 'rounded' } })}
          />
          <ModeButton
            label="Soft"
            active={config.ui.cornerStyle === 'soft'}
            onClick={() => saveWithFeedback({ ui: { ...config.ui, cornerStyle: 'soft' } })}
          />
          <ModeButton
            label="Square"
            active={config.ui.cornerStyle === 'square'}
            onClick={() => saveWithFeedback({ ui: { ...config.ui, cornerStyle: 'square' } })}
          />
        </div>
      </Row>
      <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-disabled)' }}>
        Each theme has its own corner style. "Theme" follows it; pick another to override until you switch themes.
      </div>
      <Row label="Border color">
        <div style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
          <input
            type="color"
            value={borderHex}
            onChange={(e) => saveWithFeedback({ ui: { ...config.ui, borderColor: e.target.value } })}
            title="Focused-pane border color"
            style={{
              width: '28px', height: '24px', padding: 0, cursor: 'pointer',
              background: 'transparent', border: '1px solid var(--wks-border)',
              borderRadius: 'var(--wks-radius-sm)',
            }}
          />
          {config.ui.borderColor && (
            <ModeButton
              label="Theme"
              active={false}
              onClick={() => saveWithFeedback({ ui: { ...config.ui, borderColor: '' } })}
            />
          )}
        </div>
      </Row>
      <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-disabled)' }}>
        Border around the focused pane when a tab is split. Defaults to the theme's accent; switching themes resets it.
      </div>
    </Section>
  );
};

export default AppearanceSection;
