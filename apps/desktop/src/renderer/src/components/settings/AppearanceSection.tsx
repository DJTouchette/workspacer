import React, { useCallback, useRef, useState } from 'react';
import { Check } from 'lucide-react';
import { Config } from '../../hooks/useConfig';
import {
  themes,
  toHex,
  resolveTheme,
  themeDisplayName,
  themeColorsOf,
  isCustomThemeId,
  newCustomThemeId,
  DEFAULT_THEME,
} from '../../themes';
import type { CustomTheme } from '../../themes';
import { Section, Row, ModeButton, SmallButton, SearchableSelect, inputStyle } from './primitives';
import ThemeMaker from './ThemeMaker';

interface AppearanceSectionProps {
  config: Config;
  save: (partial: Partial<Config>) => Promise<Config>;
}

/** Snap presets on the text-size slider — the same ladder the chat presets
 *  use, anchored at 100% (the app's designed size). */
const TEXT_SCALE_PRESETS: { label: string; value: number }[] = [
  { label: 'Default', value: 1.0 },
  { label: 'Medium', value: 1.15 },
  { label: 'Large', value: 1.3 },
  { label: 'XL', value: 1.5 },
];

const AppearanceSection: React.FC<AppearanceSectionProps> = ({ config, save }) => {
  const customThemes = config.ui.customThemes ?? {};
  const activeTheme = resolveTheme(config.ui.theme, customThemes);
  const borderHex = toHex(config.ui.borderColor || activeTheme.borderActive || activeTheme.accent);
  const [saved, setSaved] = useState(false);
  const savedTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  // Inline "New theme…" name prompt.
  const [naming, setNaming] = useState(false);
  const [newName, setNewName] = useState('');

  const themeOptions = React.useMemo(
    () => [
      ...Object.entries(themes).map(([id, t]) => ({
        value: id,
        label: themeDisplayName(id),
        swatch: t.accent,
        group: 'Built-in',
      })),
      ...Object.keys(customThemes).map((id) => ({
        value: id,
        label: themeDisplayName(id, customThemes),
        swatch: resolveTheme(id, customThemes).accent,
        group: 'Custom',
      })),
    ],
    [customThemes],
  );

  const saveWithFeedback = useCallback(
    async (partial: Partial<Config>) => {
      await save(partial);
      setSaved(true);
      if (savedTimerRef.current) clearTimeout(savedTimerRef.current);
      savedTimerRef.current = setTimeout(() => setSaved(false), 1500);
    },
    [save],
  );

  /** Fork the CURRENT theme (as rendered) into a new custom theme and switch
   *  to it — the editor below opens automatically. Colors are fully resolved
   *  at creation, so later built-in tweaks never restyle it. */
  const createTheme = useCallback(() => {
    const name = newName.trim() || 'My theme';
    const id = newCustomThemeId(name, customThemes);
    // Fork of a fork keeps pointing at the original built-in base.
    const base = isCustomThemeId(config.ui.theme)
      ? (customThemes[config.ui.theme]?.base ?? DEFAULT_THEME)
      : config.ui.theme || DEFAULT_THEME;
    const spec: CustomTheme = { name, base, colors: themeColorsOf(activeTheme) };
    setNaming(false);
    setNewName('');
    void saveWithFeedback({
      ui: { ...config.ui, theme: id, customThemes: { ...customThemes, [id]: spec } },
    });
  }, [newName, customThemes, config.ui, activeTheme, saveWithFeedback]);

  return (
    <Section title="Appearance">
      {/* Saved feedback chip */}
      {saved && (
        <div
          style={{
            display: 'inline-flex',
            alignItems: 'center',
            gap: '4px',
            fontSize: '0.64rem',
            color: 'var(--wks-accent)',
            background: 'var(--wks-bg-selected)',
            border: '1px solid var(--wks-accent)',
            borderRadius: '10px',
            padding: '1px 8px',
            marginBottom: '6px',
            animation: 'wks-fade-in 0.15s ease',
          }}
        >
          Saved <Check size={11} strokeWidth={2} />
        </div>
      )}
      <Row label="Theme">
        <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
          {!naming && <SmallButton label="New theme…" onClick={() => setNaming(true)} />}
          <SearchableSelect
            value={config.ui.theme}
            options={themeOptions}
            /* Switching theme re-adopts that theme's own corner style and
             * border color (overrides cleared). */
            onChange={(themeName) =>
              saveWithFeedback({
                ui: { ...config.ui, theme: themeName, cornerStyle: '', borderColor: '' },
              })
            }
          />
        </div>
      </Row>
      {naming && (
        <div style={{ display: 'flex', alignItems: 'center', gap: 6, padding: '0 0 12px' }}>
          <input
            type="text"
            autoFocus
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') createTheme();
              if (e.key === 'Escape') {
                setNaming(false);
                setNewName('');
              }
            }}
            placeholder="Theme name"
            spellCheck={false}
            style={{ ...inputStyle, flex: 1, minWidth: 0 }}
          />
          <SmallButton label="Create" primary onClick={createTheme} />
          <SmallButton
            label="Cancel"
            onClick={() => {
              setNaming(false);
              setNewName('');
            }}
          />
          <span style={{ fontSize: '0.7rem', color: 'var(--wks-text-disabled)', flexShrink: 0 }}>
            copies {themeDisplayName(config.ui.theme || DEFAULT_THEME, customThemes)}
          </span>
        </div>
      )}
      {isCustomThemeId(config.ui.theme) && customThemes[config.ui.theme] && (
        <ThemeMaker config={config} save={save} themeId={config.ui.theme} />
      )}
      <Row label="Text size">
        <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
          <input
            type="range"
            min={0.8}
            max={1.5}
            step={0.05}
            value={config.ui.uiFontScale ?? 1.0}
            onChange={(e) => save({ ui: { ...config.ui, uiFontScale: Number(e.target.value) } })}
            list="wks-text-scale-presets"
            style={{ width: 180, accentColor: 'var(--wks-accent)' }}
          />
          {/* Snap points at the preset scales. */}
          <datalist id="wks-text-scale-presets">
            {TEXT_SCALE_PRESETS.map((p) => (
              <option key={p.value} value={p.value} label={p.label} />
            ))}
          </datalist>
          <span
            style={{
              fontSize: '0.72rem',
              fontVariantNumeric: 'tabular-nums',
              color: 'var(--wks-text-secondary)',
              width: 42,
              textAlign: 'right',
            }}
          >
            {Math.round((config.ui.uiFontScale ?? 1.0) * 100)}%
          </span>
          <div style={{ display: 'flex', gap: 4 }}>
            {TEXT_SCALE_PRESETS.map((p) => (
              <ModeButton
                key={p.value}
                label={p.label}
                active={(config.ui.uiFontScale ?? 1.0) === p.value}
                onClick={() => save({ ui: { ...config.ui, uiFontScale: p.value } })}
              />
            ))}
          </div>
        </div>
      </Row>
      <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-disabled)' }}>
        Scales all text in the app (the terminal view and chat keep their own additional size
        settings). Also on Ctrl/Cmd + = , - and 0 to reset.
      </div>

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
        Each theme has its own corner style. "Theme" follows it; pick another to override until you
        switch themes.
      </div>
      <Row label="Border color">
        <div style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
          <input
            type="color"
            value={borderHex}
            onChange={(e) =>
              saveWithFeedback({ ui: { ...config.ui, borderColor: e.target.value } })
            }
            title="Focused-pane border color"
            style={{
              width: '28px',
              height: '24px',
              padding: 0,
              cursor: 'pointer',
              background: 'transparent',
              border: '1px solid var(--wks-border)',
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
        Border around the focused pane when a tab is split. Defaults to the theme's accent;
        switching themes resets it.
      </div>
    </Section>
  );
};

export default AppearanceSection;
