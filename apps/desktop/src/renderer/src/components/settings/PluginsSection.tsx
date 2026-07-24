import React, { useEffect, useState } from 'react';
import { ChevronLeft, ChevronRight } from 'lucide-react';
import { Puzzle } from '../icons';
import { usePlugins } from '../../hooks/usePlugins';
import { SECRET_PLACEHOLDER } from '../../types/plugin';
import type { PluginManifest, PluginSettingDef } from '../../types/plugin';
import { Section, Row, CheckRow, ModeButton, SmallButton, inputStyle } from './primitives';

/**
 * Plugin settings, two-level: a list page of every installed plugin (the
 * 'plugins' settings section) and a per-plugin detail page ('plugin:<id>')
 * holding that plugin's declared settings — instead of every plugin's controls
 * dumped inline into one section. Install / enable / update stays in the
 * Plugins pane; this is purely the configuration surface.
 */

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
const PluginSettingsControls: React.FC<{ plugin: PluginManifest }> = ({ plugin }) => {
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
    <>
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
    </>
  );
};

/** Leading glyph for a plugin row: its first pane's icon (user-supplied emoji,
 *  rendered as given) or the Puzzle icon as the code-side fallback. */
const PluginGlyph: React.FC<{ plugin: PluginManifest }> = ({ plugin }) => {
  const emoji = plugin.panes?.find((p) => p.icon)?.icon;
  return (
    <span
      aria-hidden
      style={{
        width: 28,
        height: 28,
        flexShrink: 0,
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        borderRadius: 'var(--wks-radius-sm)',
        background: 'var(--wks-bg-raised)',
        border: '1px solid var(--wks-border-subtle)',
        color: 'var(--wks-text-muted)',
        fontSize: '0.9rem',
      }}
    >
      {emoji || <Puzzle size={14} strokeWidth={1.75} />}
    </span>
  );
};

const DisabledChip: React.FC = () => (
  <span
    style={{
      fontSize: '0.6rem',
      fontWeight: 600,
      textTransform: 'uppercase',
      letterSpacing: '0.06em',
      padding: '2px 8px',
      borderRadius: 'var(--wks-radius-pill)',
      color: 'var(--wks-warning)',
      background: 'color-mix(in srgb, var(--wks-warning) 12%, transparent)',
      flexShrink: 0,
    }}
  >
    Disabled
  </span>
);

/** The 'plugins' section: every installed plugin as a clickable row. */
export const PluginsListSection: React.FC<{ onOpen: (pluginId: string) => void }> = ({
  onOpen,
}) => {
  const { plugins } = usePlugins();
  return (
    <Section title="Plugins">
      <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-disabled)', marginBottom: 12 }}>
        Installed plugins — select one to configure it. Installing, updating and enabling live in
        the Plugins pane.
      </div>
      {plugins.length === 0 && (
        <div style={{ fontSize: '0.8rem', color: 'var(--wks-text-faint)', padding: '16px 0' }}>
          No plugins installed. Open the Plugins pane to browse the catalog.
        </div>
      )}
      {plugins.map((p) => {
        const count = p.settings?.length ?? 0;
        return (
          <button
            key={p.id}
            onClick={() => onOpen(p.id)}
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 10,
              width: '100%',
              padding: '10px 12px',
              marginBottom: 8,
              textAlign: 'left',
              fontFamily: 'inherit',
              background: 'transparent',
              border: '1px solid var(--wks-border-subtle)',
              borderRadius: 'var(--wks-radius-md)',
              cursor: 'pointer',
              transition: 'background 0.12s, border-color 0.12s',
              opacity: p.disabled ? 0.6 : 1,
            }}
            onMouseEnter={(e) => {
              e.currentTarget.style.background = 'var(--wks-bg-hover)';
            }}
            onMouseLeave={(e) => {
              e.currentTarget.style.background = 'transparent';
            }}
          >
            <PluginGlyph plugin={p} />
            <span style={{ flex: 1, minWidth: 0 }}>
              <span
                style={{
                  display: 'block',
                  fontSize: '0.8rem',
                  fontWeight: 500,
                  color: 'var(--wks-text-primary)',
                  overflow: 'hidden',
                  textOverflow: 'ellipsis',
                  whiteSpace: 'nowrap',
                }}
              >
                {p.name}
              </span>
              <span
                style={{
                  display: 'block',
                  fontSize: '0.66rem',
                  fontFamily: 'var(--wks-font-mono)',
                  color: 'var(--wks-text-faint)',
                  overflow: 'hidden',
                  textOverflow: 'ellipsis',
                  whiteSpace: 'nowrap',
                }}
              >
                {p.id}
                {p.version ? ' · v' + p.version : ''}
              </span>
            </span>
            {p.disabled && <DisabledChip />}
            <span style={{ fontSize: '0.72rem', color: 'var(--wks-text-muted)', flexShrink: 0 }}>
              {count > 0 ? count + (count === 1 ? ' setting' : ' settings') : 'no settings'}
            </span>
            <span
              style={{
                display: 'flex',
                alignItems: 'center',
                color: 'var(--wks-text-faint)',
                flexShrink: 0,
              }}
            >
              <ChevronRight size={12} strokeWidth={2} />
            </span>
          </button>
        );
      })}
    </Section>
  );
};

/** The 'plugin:<id>' section: one plugin's settings page. */
export const PluginDetailSection: React.FC<{ pluginId: string; onBack: () => void }> = ({
  pluginId,
  onBack,
}) => {
  const { plugins } = usePlugins();
  const plugin = plugins.find((p) => p.id === pluginId);

  // Uninstalled (or hub restarted without it) while open — fall back to the list.
  useEffect(() => {
    if (plugins.length > 0 && !plugin) onBack();
  }, [plugins.length, plugin, onBack]);

  if (!plugin) return null;

  return (
    <div>
      <button
        onClick={onBack}
        style={{
          display: 'inline-flex',
          alignItems: 'center',
          gap: 4,
          padding: '4px 0',
          marginBottom: 10,
          fontSize: '0.72rem',
          fontFamily: 'inherit',
          fontWeight: 600,
          background: 'none',
          border: 'none',
          cursor: 'pointer',
          color: 'var(--wks-text-muted)',
        }}
        onMouseEnter={(e) => {
          e.currentTarget.style.color = 'var(--wks-accent-text)';
        }}
        onMouseLeave={(e) => {
          e.currentTarget.style.color = 'var(--wks-text-muted)';
        }}
      >
        <ChevronLeft size={12} strokeWidth={2} />
        All plugins
      </button>

      <Section title={plugin.name}>
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 8,
            marginBottom: plugin.settings?.length ? 4 : 0,
          }}
        >
          <PluginGlyph plugin={plugin} />
          <span
            style={{
              fontSize: '0.66rem',
              fontFamily: 'var(--wks-font-mono)',
              color: 'var(--wks-text-faint)',
            }}
          >
            {plugin.id}
            {plugin.version ? ' · v' + plugin.version : ''}
          </span>
          {plugin.disabled && <DisabledChip />}
        </div>
        {plugin.settings?.length ? (
          <PluginSettingsControls plugin={plugin} />
        ) : (
          <div style={{ fontSize: '0.8rem', color: 'var(--wks-text-faint)', padding: '16px 0' }}>
            This plugin has no settings.
          </div>
        )}
        {(plugin.settings?.length ?? 0) > 0 && (
          <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-disabled)', marginTop: 8 }}>
            Changes apply to open plugin panes live; plugins with a sidecar restart to pick them up.
          </div>
        )}
      </Section>
    </div>
  );
};
