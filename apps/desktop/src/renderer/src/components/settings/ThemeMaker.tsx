import React, { useCallback, useEffect, useRef, useState } from 'react';
import { Config } from '../../hooks/useConfig';
import type { CustomTheme, TerminalTheme, Theme, ThemeColors } from '../../themes';
import { newCustomThemeId, themes, toHex } from '../../themes';
import { SmallButton, inputStyle } from './primitives';

interface ThemeMakerProps {
  config: Config;
  save: (partial: Partial<Config>) => Promise<Config>;
  /** The custom theme being edited ('custom:<slug>' — must exist in config). */
  themeId: string;
}

/** Flat Theme color tokens, grouped for the editor. */
type FlatToken = Exclude<keyof ThemeColors, 'terminal'>;

const TOKEN_GROUPS: Array<{ title: string; tokens: FlatToken[]; collapsed?: boolean }> = [
  {
    title: 'Surfaces',
    tokens: [
      'bgBase',
      'bgRaised',
      'bgSurface',
      'bgElevated',
      'bgHeader',
      'bgInput',
      'bgHover',
      'bgSelected',
      'bgTerminal',
      'claudeBg',
    ],
  },
  { title: 'Borders', tokens: ['border', 'borderSubtle', 'borderInput'] },
  {
    title: 'Text',
    tokens: [
      'textPrimary',
      'textSecondary',
      'textTertiary',
      'textMuted',
      'textFaint',
      'textDisabled',
    ],
  },
  { title: 'Accent', tokens: ['accent', 'accentText', 'accentGlow', 'accentBg', 'borderActive'] },
  { title: 'Status', tokens: ['success', 'error', 'warning', 'purple', 'busy'] },
  {
    title: 'Chrome & Chat',
    collapsed: true,
    tokens: [
      'overlay',
      'shadow',
      'scrollbarThumb',
      'scrollbarHover',
      'claudeUserBubble',
      'claudeUserBorder',
      'claudeDivider',
      'claudeBorder',
      'claudeBorderSubtle',
    ],
  },
];

const TERMINAL_TOKENS: Array<keyof TerminalTheme> = [
  'background',
  'foreground',
  'cursor',
  'cursorAccent',
  'selectionBackground',
  'selectionForeground',
  'black',
  'red',
  'green',
  'yellow',
  'blue',
  'magenta',
  'cyan',
  'white',
  'brightBlack',
  'brightRed',
  'brightGreen',
  'brightYellow',
  'brightBlue',
  'brightMagenta',
  'brightCyan',
  'brightWhite',
];

/** "bgBase" → "Bg Base", "claudeUserBubble" → "Claude User Bubble". */
function tokenLabel(token: string): string {
  const spaced = token.replace(/([a-z])([A-Z])/g, '$1 $2');
  return spaced.charAt(0).toUpperCase() + spaced.slice(1);
}

/** One editable token: native color swatch + free-form CSS color text input.
 *  The text input accepts anything (rgba(), color-mix(), …); the swatch shows
 *  a best-effort hex of the current value. */
const ColorRow: React.FC<{
  label: string;
  value: string;
  onChange: (v: string) => void;
}> = ({ label, value, onChange }) => (
  <div
    style={{
      display: 'flex',
      alignItems: 'center',
      gap: 8,
      padding: '3px 0',
    }}
  >
    <span
      style={{
        flex: 1,
        fontSize: '0.74rem',
        color: 'var(--wks-text-muted)',
        overflow: 'hidden',
        textOverflow: 'ellipsis',
        whiteSpace: 'nowrap',
      }}
    >
      {label}
    </span>
    <input
      type="color"
      value={toHex(value)}
      onChange={(e) => onChange(e.target.value)}
      title={label}
      style={{
        width: 26,
        height: 22,
        padding: 0,
        cursor: 'pointer',
        background: 'transparent',
        border: '1px solid var(--wks-border)',
        borderRadius: 'var(--wks-radius-sm)',
        flexShrink: 0,
      }}
    />
    <input
      type="text"
      value={value}
      onChange={(e) => onChange(e.target.value)}
      spellCheck={false}
      style={{
        ...inputStyle,
        width: 190,
        height: 24,
        fontSize: '0.72rem',
        fontFamily: 'ui-monospace, monospace',
        flexShrink: 0,
      }}
    />
  </div>
);

/** Quiet collapsible group of color rows. */
const TokenGroup: React.FC<{
  title: string;
  defaultOpen: boolean;
  children: React.ReactNode;
}> = ({ title, defaultOpen, children }) => {
  const [open, setOpen] = useState(defaultOpen);
  return (
    <div style={{ borderTop: '1px solid var(--wks-border-subtle)' }}>
      <button
        onClick={() => setOpen((v) => !v)}
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 6,
          width: '100%',
          padding: '8px 0',
          background: 'transparent',
          border: 'none',
          cursor: 'pointer',
          fontFamily: 'inherit',
          fontSize: '0.68rem',
          fontWeight: 700,
          letterSpacing: '0.08em',
          textTransform: 'uppercase',
          color: 'var(--wks-text-faint)',
          textAlign: 'left',
        }}
      >
        <span style={{ fontSize: '0.6rem', width: 10 }}>{open ? '▾' : '▸'}</span>
        {title}
      </button>
      {open && <div style={{ paddingBottom: 8 }}>{children}</div>}
    </div>
  );
};

/**
 * Inline editor for the selected custom theme. Edits are applied live — the
 * draft is saved to config (debounced) on every change, and useTheme re-paints
 * the app from config, so the whole app is the preview.
 */
const ThemeMaker: React.FC<ThemeMakerProps> = ({ config, save, themeId }) => {
  const customThemes = config.ui.customThemes ?? {};
  const spec = customThemes[themeId];

  const [draft, setDraft] = useState<CustomTheme | null>(spec ?? null);
  const draftRef = useRef(draft);
  draftRef.current = draft;
  const configRef = useRef(config);
  configRef.current = config;
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Re-seed the draft when the edited theme changes (not on every config echo
  // of our own debounced saves — that would clobber in-flight typing).
  useEffect(() => {
    setDraft(config.ui.customThemes?.[themeId] ?? null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [themeId]);

  useEffect(() => {
    return () => {
      if (timerRef.current) clearTimeout(timerRef.current);
    };
  }, []);

  // Apply an edit: update the local draft immediately (input echo) and save it
  // to config debounced ~300ms. The saved draft and theme id are captured in
  // the closure, so a pending save can never write one theme's colors under
  // another theme's id.
  const update = useCallback(
    (mut: (d: CustomTheme) => CustomTheme) => {
      const prev = draftRef.current;
      if (!prev) return;
      const next = mut(prev);
      draftRef.current = next;
      setDraft(next);
      if (timerRef.current) clearTimeout(timerRef.current);
      timerRef.current = setTimeout(() => {
        timerRef.current = null;
        const cfg = configRef.current;
        void save({
          ui: { ...cfg.ui, customThemes: { ...cfg.ui.customThemes, [themeId]: next } },
        });
      }, 300);
    },
    [save, themeId],
  );

  if (!spec || !draft) return null;

  // Effective values for the inputs: the stored colors backfilled from the
  // base theme, exactly like resolveTheme does.
  const base: Theme = themes[draft.base ?? ''] ?? themes.dark;
  const flat = { ...base, ...draft.colors } as Record<FlatToken, string | undefined>;
  const term: TerminalTheme = { ...base.terminal, ...(draft.colors.terminal ?? {}) };
  // Optional tokens fall back the same way cssVarsOf() does, so the inputs
  // show the color that's actually in effect.
  flat.borderActive = flat.borderActive ?? flat.accent;
  flat.busy = flat.busy ?? flat.accent;
  flat.purple = flat.purple ?? '#c084fc';

  const setToken = (token: FlatToken, value: string) =>
    update((d) => ({ ...d, colors: { ...d.colors, [token]: value } }));
  const setTerminalToken = (token: keyof TerminalTheme, value: string) =>
    update((d) => ({
      ...d,
      colors: { ...d.colors, terminal: { ...d.colors.terminal, [token]: value } },
    }));

  const duplicate = () => {
    if (timerRef.current) clearTimeout(timerRef.current);
    const cfg = configRef.current;
    const existing = { ...cfg.ui.customThemes, [themeId]: draftRef.current ?? spec };
    const name = `${draft.name} copy`;
    const id = newCustomThemeId(name, existing);
    const copy: CustomTheme = {
      name,
      base: draft.base,
      colors: JSON.parse(JSON.stringify(draft.colors)) as ThemeColors,
    };
    void save({
      ui: { ...cfg.ui, theme: id, customThemes: { ...existing, [id]: copy } },
    });
  };

  const remove = () => {
    if (timerRef.current) clearTimeout(timerRef.current);
    const cfg = configRef.current;
    const rest = { ...cfg.ui.customThemes };
    delete rest[themeId];
    const fallback = draft.base && themes[draft.base] ? draft.base : 'dark';
    void save({
      ui: { ...cfg.ui, theme: fallback, customThemes: rest, cornerStyle: '', borderColor: '' },
    });
  };

  return (
    <div
      style={{
        marginTop: 4,
        padding: '10px 12px',
        background: 'var(--wks-bg-raised)',
        border: '1px solid var(--wks-border-subtle)',
        borderRadius: 'var(--wks-radius-md)',
      }}
    >
      {/* Header: rename inline + duplicate/delete */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 10 }}>
        <input
          type="text"
          value={draft.name}
          onChange={(e) => update((d) => ({ ...d, name: e.target.value }))}
          placeholder="Theme name"
          spellCheck={false}
          style={{ ...inputStyle, flex: 1, minWidth: 0 }}
        />
        <SmallButton label="Duplicate" onClick={duplicate} />
        <SmallButton label="Delete" danger onClick={remove} />
      </div>
      <div style={{ fontSize: '0.7rem', color: 'var(--wks-text-disabled)', marginBottom: 6 }}>
        Edits apply live — the app is the preview. Text fields take any CSS color (hex, rgba(),
        color-mix()). Based on {draft.base || 'dark'}.
      </div>

      {TOKEN_GROUPS.map((group) => (
        <TokenGroup key={group.title} title={group.title} defaultOpen={!group.collapsed}>
          {group.tokens.map((token) => (
            <ColorRow
              key={token}
              label={tokenLabel(token)}
              value={flat[token] ?? ''}
              onChange={(v) => setToken(token, v)}
            />
          ))}
        </TokenGroup>
      ))}

      <TokenGroup title="Terminal" defaultOpen={false}>
        {TERMINAL_TOKENS.map((token) => (
          <ColorRow
            key={token}
            label={tokenLabel(token)}
            value={term[token] ?? ''}
            onChange={(v) => setTerminalToken(token, v)}
          />
        ))}
      </TokenGroup>
    </div>
  );
};

export default ThemeMaker;
