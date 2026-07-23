import React, { useEffect, useState } from 'react';
import { usePlugins } from '../../hooks/usePlugins';
import { SECRET_PLACEHOLDER } from '../../types/plugin';
import type { PluginManifest, PluginSettingDef } from '../../types/plugin';
import { Section, Row, CheckRow, ModeButton, SmallButton, inputStyle } from './primitives';

/**
 * Write-only input for a secret setting (PAT/API key). The hub never returns
 * the stored value — reads report SECRET_PLACEHOLDER — so this renders a
 * masked field that is empty until the user types a replacement. Commits on
 * blur/Enter rather than per keystroke: every save restarts the plugin's
 * sidecar, and a half-typed token must never be persisted.
 */
const SecretInput: React.FC<{ stored: boolean; onCommit: (value: string) => void }> = ({
  stored,
  onCommit,
}) => {
  const [draft, setDraft] = useState('');
  const commit = () => {
    if (draft.trim()) {
      onCommit(draft.trim());
      setDraft('');
    }
  };
  return (
    <div style={{ display: 'flex', gap: 6, alignItems: 'center' }}>
      <input
        type="password"
        autoComplete="off"
        spellCheck={false}
        style={{ ...inputStyle, width: 160 }}
        value={draft}
        placeholder={stored ? '•••••••• (set — type to replace)' : 'not set'}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === 'Enter') commit();
        }}
      />
      {stored && <SmallButton label="Clear" onClick={() => onCommit('')} />}
    </div>
  );
};

/** Renders + persists one plugin's declared settings. The host (hub) returns
 *  values already merged over the plugin's manifest defaults, so every declared
 *  setting that has a default is present; the `s.default` fallback below only
 *  covers settings with no declared default. Edits made from web/remote arrive
 *  on the plugin-settings-changed channel and update the controls live. */
const PluginSettings: React.FC<{ plugin: PluginManifest }> = ({ plugin }) => {
  const [values, setValues] = useState<Record<string, unknown>>({});

  useEffect(() => {
    let alive = true;
    window.electronAPI.getPluginSettings?.(plugin.id).then((v) => {
      if (alive) setValues(v || {});
    });
    const off = window.electronAPI.onPluginSettingsChanged?.((changedId, next) => {
      if (alive && changedId === plugin.id) setValues(next || {});
    });
    return () => {
      alive = false;
      off?.();
    };
  }, [plugin.id]);

  const update = (key: string, value: unknown) => {
    setValues((prev) => ({ ...prev, [key]: value }));
    window.electronAPI.setPluginSettings?.(plugin.id, { [key]: value });
  };
  const valueFor = (s: PluginSettingDef) => (s.key in values ? values[s.key] : s.default);

  const control = (s: PluginSettingDef) => {
    const v = valueFor(s);
    switch (s.type) {
      case 'boolean':
        return (
          <CheckRow
            key={s.key}
            label={s.label}
            checked={!!v}
            onChange={(nv) => update(s.key, nv)}
          />
        );
      case 'number':
        return (
          <Row key={s.key} label={s.label}>
            <input
              type="number"
              style={{ ...inputStyle, width: 80 }}
              value={Number(v ?? 0)}
              onChange={(e) => update(s.key, Number(e.target.value))}
            />
          </Row>
        );
      case 'select':
        return (
          <Row key={s.key} label={s.label}>
            <div style={{ display: 'flex', gap: 4, flexWrap: 'wrap' }}>
              {(s.options ?? []).map((opt) => (
                <ModeButton
                  key={opt}
                  label={opt}
                  active={String(v) === opt}
                  onClick={() => update(s.key, opt)}
                />
              ))}
            </div>
          </Row>
        );
      default: // string
        if (s.secret) {
          // Set when the hub reports the redaction sentinel (or, against an
          // older hub that predates redaction, any non-empty value).
          const stored = v === SECRET_PLACEHOLDER || (typeof v === 'string' && v !== '');
          return (
            <Row key={s.key} label={s.label}>
              <SecretInput stored={stored} onCommit={(nv) => update(s.key, nv)} />
            </Row>
          );
        }
        return (
          <Row key={s.key} label={s.label}>
            <input
              style={{ ...inputStyle, width: 160 }}
              value={String(v ?? '')}
              spellCheck={false}
              onChange={(e) => update(s.key, e.target.value)}
            />
          </Row>
        );
    }
  };

  return (
    <div style={{ marginBottom: 10 }}>
      <div style={{ fontSize: '0.66rem', color: 'var(--wks-text-secondary)', margin: '8px 0 2px' }}>
        {plugin.name}
      </div>
      {(plugin.settings ?? []).map((s) => (
        <React.Fragment key={s.key}>
          {control(s)}
          {s.help && (
            <div
              style={{ fontSize: '0.72rem', color: 'var(--wks-text-disabled)', marginBottom: 2 }}
            >
              {s.help}
            </div>
          )}
        </React.Fragment>
      ))}
    </div>
  );
};

/** Settings section aggregating every installed plugin that declares settings. */
const PluginsSection: React.FC = () => {
  const { plugins } = usePlugins();
  const configurable = plugins.filter((p) => (p.settings?.length ?? 0) > 0 && !p.disabled);
  if (!configurable.length) return null;
  return (
    <Section title="Plugins">
      <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-disabled)', marginBottom: 4 }}>
        Options contributed by installed plugins. Changes apply to open plugin panes live.
      </div>
      {configurable.map((p) => (
        <PluginSettings key={p.id} plugin={p} />
      ))}
    </Section>
  );
};

export default PluginsSection;
