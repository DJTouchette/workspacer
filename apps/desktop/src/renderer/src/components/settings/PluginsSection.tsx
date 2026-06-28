import React, { useEffect, useState } from 'react';
import { usePlugins } from '../../hooks/usePlugins';
import type { PluginManifest, PluginSettingDef } from '../../types/plugin';
import { Section, Row, CheckRow, ModeButton, inputStyle } from './primitives';

/** Renders + persists one plugin's declared settings. Saved values are an
 *  overlay on the plugin's own defaults, so an unset control shows the default. */
const PluginSettings: React.FC<{ plugin: PluginManifest }> = ({ plugin }) => {
  const [values, setValues] = useState<Record<string, unknown>>({});

  useEffect(() => {
    let alive = true;
    window.electronAPI.getPluginSettings?.(plugin.id).then((v) => { if (alive) setValues(v || {}); });
    return () => { alive = false; };
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
        return <CheckRow key={s.key} label={s.label} checked={!!v} onChange={(nv) => update(s.key, nv)} />;
      case 'number':
        return (
          <Row key={s.key} label={s.label}>
            <input type="number" style={{ ...inputStyle, width: 80 }} value={Number(v ?? 0)}
              onChange={(e) => update(s.key, Number(e.target.value))} />
          </Row>
        );
      case 'select':
        return (
          <Row key={s.key} label={s.label}>
            <div style={{ display: 'flex', gap: 4, flexWrap: 'wrap' }}>
              {(s.options ?? []).map((opt) => (
                <ModeButton key={opt} label={opt} active={String(v) === opt} onClick={() => update(s.key, opt)} />
              ))}
            </div>
          </Row>
        );
      default: // string
        return (
          <Row key={s.key} label={s.label}>
            <input style={{ ...inputStyle, width: 160 }} value={String(v ?? '')} spellCheck={false}
              onChange={(e) => update(s.key, e.target.value)} />
          </Row>
        );
    }
  };

  return (
    <div style={{ marginBottom: 10 }}>
      <div style={{ fontSize: '0.62rem', color: 'var(--wks-text-secondary)', margin: '8px 0 2px' }}>{plugin.name}</div>
      {(plugin.settings ?? []).map((s) => (
        <React.Fragment key={s.key}>
          {control(s)}
          {s.help && <div style={{ fontSize: '0.55rem', color: 'var(--wks-text-disabled)', marginBottom: 2 }}>{s.help}</div>}
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
      <div style={{ fontSize: '0.55rem', color: 'var(--wks-text-disabled)', marginBottom: 4 }}>
        Options contributed by installed plugins. Changes apply to open plugin panes live.
      </div>
      {configurable.map((p) => <PluginSettings key={p.id} plugin={p} />)}
    </Section>
  );
};

export default PluginsSection;
