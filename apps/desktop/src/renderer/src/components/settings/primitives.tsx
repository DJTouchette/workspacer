import React from 'react';
import { fuzzyScoreAny } from '../../lib/fuzzy';

export const inputStyle: React.CSSProperties = {
  height: '30px',
  padding: '0 10px',
  fontSize: '0.78rem',
  backgroundColor: 'var(--wks-bg-input)',
  color: 'var(--wks-text-secondary)',
  border: '1px solid var(--wks-border)',
  borderRadius: 'var(--wks-radius-sm)',
  outline: 'none',
  fontFamily: 'inherit',
  boxSizing: 'border-box' as const,
};

export function SmallButton({
  label,
  onClick,
  primary,
  danger,
}: {
  label: string;
  onClick: () => void;
  primary?: boolean;
  danger?: boolean;
}) {
  return (
    <button
      onClick={(e) => {
        e.stopPropagation();
        onClick();
      }}
      style={{
        padding: '4px 12px',
        fontSize: '0.72rem',
        fontFamily: 'inherit',
        fontWeight: 600,
        backgroundColor: primary ? 'var(--wks-accent)' : 'transparent',
        color: danger
          ? 'var(--wks-error)'
          : primary
            ? 'var(--wks-text-on-accent, #fff)'
            : 'var(--wks-text-muted)',
        border: primary ? '1px solid var(--wks-accent)' : '1px solid var(--wks-border-subtle)',
        borderRadius: 'var(--wks-radius-pill, 999px)',
        cursor: 'pointer',
        height: 'auto',
        lineHeight: 1.4,
        margin: 0,
        width: 'auto',
        transition: 'background 0.12s, color 0.12s, border-color 0.12s',
      }}
    >
      {label}
    </button>
  );
}

export function Section({
  title,
  children,
  id,
}: {
  title: string;
  children: React.ReactNode;
  id?: string;
}) {
  return (
    <div id={id} style={{ marginBottom: '32px', animation: 'wks-fade-in 0.18s ease' }}>
      <div
        style={{
          fontSize: '0.58rem',
          fontWeight: 600,
          color: 'var(--wks-text-faint)',
          textTransform: 'uppercase',
          letterSpacing: '0.08em',
          marginBottom: '14px',
          paddingBottom: '10px',
          borderBottom: '1px solid var(--wks-border-subtle)',
        }}
      >
        {title}
      </div>
      <div
        className="wks-settings-section-body"
        style={{ display: 'flex', flexDirection: 'column' }}
      >
        {children}
      </div>
    </div>
  );
}

/** A divided settings row: bold label left, control right. */
const rowFrame: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'space-between',
  gap: '16px',
  padding: '16px 0',
  borderTop: '1px solid var(--wks-border-subtle)',
};
const rowLabel: React.CSSProperties = {
  fontSize: '0.85rem',
  fontWeight: 500,
  color: 'var(--wks-text-primary)',
  flexShrink: 0,
};

export function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div style={rowFrame}>
      <span style={rowLabel}>{label}</span>
      {children}
    </div>
  );
}

export function CheckRow({
  label,
  checked,
  onChange,
  disabled,
}: {
  label: string;
  checked: boolean;
  onChange: (v: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <label
      style={{
        ...rowFrame,
        cursor: disabled ? 'default' : 'pointer',
        opacity: disabled ? 0.45 : 1,
      }}
    >
      <span style={rowLabel}>{label}</span>
      {/* Toggle switch. The real checkbox stays in the tree (visually hidden,
          not display:none) so keyboard focus, Space toggling and form
          semantics keep working. */}
      <span style={{ position: 'relative', display: 'inline-flex', flexShrink: 0 }}>
        <input
          type="checkbox"
          checked={checked}
          disabled={disabled}
          onChange={(e) => onChange(e.target.checked)}
          style={{
            position: 'absolute',
            inset: 0,
            opacity: 0,
            margin: 0,
            cursor: disabled ? 'default' : 'pointer',
          }}
        />
        <span
          aria-hidden
          style={{
            width: 32,
            height: 18,
            borderRadius: 'var(--wks-radius-pill, 999px)',
            background: checked
              ? 'var(--wks-accent)'
              : 'color-mix(in srgb, var(--wks-text-faint) 25%, transparent)',
            border: '1px solid',
            borderColor: checked ? 'var(--wks-accent)' : 'var(--wks-border)',
            transition: 'background 0.15s, border-color 0.15s',
            boxSizing: 'border-box',
            display: 'inline-block',
            position: 'relative',
            pointerEvents: 'none',
          }}
        >
          <span
            style={{
              position: 'absolute',
              top: 2,
              left: checked ? 16 : 2,
              width: 12,
              height: 12,
              borderRadius: '50%',
              background: checked ? 'var(--wks-text-on-accent, #fff)' : 'var(--wks-text-muted)',
              transition: 'left 0.15s ease, background 0.15s',
            }}
          />
        </span>
      </span>
    </label>
  );
}

export function ModeButton({
  label,
  active,
  onClick,
}: {
  label: string;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      style={{
        padding: '4px 14px',
        fontSize: '0.75rem',
        fontFamily: 'inherit',
        fontWeight: 600,
        backgroundColor: active ? 'var(--wks-accent-bg, rgba(99,102,241,0.14))' : 'transparent',
        color: active ? 'var(--wks-accent-text, var(--wks-accent))' : 'var(--wks-text-muted)',
        border: '1px solid',
        borderColor: active
          ? 'color-mix(in srgb, var(--wks-accent) 45%, transparent)'
          : 'var(--wks-border-subtle)',
        borderRadius: 'var(--wks-radius-pill, 999px)',
        cursor: 'pointer',
        height: '28px',
        lineHeight: 1,
        margin: 0,
        width: 'auto',
        transition: 'background 0.12s, color 0.12s, border-color 0.12s',
      }}
    >
      {label}
    </button>
  );
}

export interface SelectOption {
  value: string;
  label: string;
  /** Optional color chip shown left of the label. */
  swatch?: string;
  /** Optional group heading (optgroup-style): a quiet header row is rendered
   *  above the first option of each run of consecutive equal groups. */
  group?: string;
}

/**
 * A compact, searchable single-select (combobox). Click to open a popover with
 * a filter box + keyboard-navigable list.
 */
export function SearchableSelect({
  value,
  options,
  onChange,
  placeholder,
  width = 190,
}: {
  value: string;
  options: SelectOption[];
  onChange: (value: string) => void;
  placeholder?: string;
  width?: number;
}) {
  const [open, setOpen] = React.useState(false);
  const [query, setQuery] = React.useState('');
  const [highlight, setHighlight] = React.useState(0);
  const rootRef = React.useRef<HTMLDivElement>(null);
  const inputRef = React.useRef<HTMLInputElement>(null);
  const listRef = React.useRef<HTMLDivElement>(null);

  const selected = options.find((o) => o.value === value);
  const filtered = React.useMemo(() => {
    const q = query.trim();
    if (!q) return options;
    // Fuzzy match on label + value, ranked best-first (stable within ties).
    return options
      .map((o) => ({ o, score: fuzzyScoreAny(q, [o.label, o.value]) }))
      .filter((x) => x.score > -Infinity)
      .sort((a, b) => b.score - a.score)
      .map((x) => x.o);
  }, [options, query]);

  React.useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener('mousedown', onDoc);
    return () => document.removeEventListener('mousedown', onDoc);
  }, [open]);

  React.useEffect(() => {
    if (!open) return;
    setQuery('');
    const idx = options.findIndex((o) => o.value === value);
    setHighlight(idx >= 0 ? idx : 0);
    const t = setTimeout(() => inputRef.current?.focus(), 0);
    return () => clearTimeout(t);
  }, [open]); // eslint-disable-line react-hooks/exhaustive-deps

  React.useEffect(() => {
    setHighlight((h) => Math.min(h, Math.max(0, filtered.length - 1)));
  }, [filtered.length]);

  React.useEffect(() => {
    if (!open) return;
    // Group headers share the list container, so target options by index.
    const el = listRef.current?.querySelector(`[data-opt-idx="${highlight}"]`) as
      HTMLElement | undefined;
    el?.scrollIntoView({ block: 'nearest' });
  }, [highlight, open]);

  const choose = (v: string) => {
    onChange(v);
    setOpen(false);
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setHighlight((h) => Math.min(filtered.length - 1, h + 1));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setHighlight((h) => Math.max(0, h - 1));
    } else if (e.key === 'Enter') {
      e.preventDefault();
      const o = filtered[highlight];
      if (o) choose(o.value);
    } else if (e.key === 'Escape') {
      e.preventDefault();
      setOpen(false);
    }
  };

  const swatchDot = (color: string) => (
    <span
      style={{
        width: 9,
        height: 9,
        borderRadius: '50%',
        background: color,
        flexShrink: 0,
        border: '1px solid rgba(255,255,255,0.15)',
      }}
    />
  );

  return (
    <div ref={rootRef} style={{ position: 'relative', width }}>
      <button
        onClick={() => setOpen((o) => !o)}
        style={{
          ...inputStyle,
          width: '100%',
          display: 'flex',
          alignItems: 'center',
          gap: 7,
          cursor: 'pointer',
          textAlign: 'left',
          borderColor: open ? 'var(--wks-accent)' : 'var(--wks-border)',
        }}
      >
        {selected?.swatch && swatchDot(selected.swatch)}
        <span
          style={{
            flex: 1,
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
            color: selected ? 'var(--wks-text-secondary)' : 'var(--wks-text-faint)',
          }}
        >
          {selected ? selected.label : placeholder || 'Select…'}
        </span>
        <span style={{ color: 'var(--wks-text-faint)', fontSize: '0.65rem', flexShrink: 0 }}>
          ▾
        </span>
      </button>

      {open && (
        <div
          style={{
            position: 'absolute',
            top: 'calc(100% + 4px)',
            right: 0,
            width: Math.max(width, 210),
            zIndex: 100,
            background: 'var(--wks-bg-elevated)',
            border: '1px solid var(--wks-border)',
            borderRadius: 'var(--wks-radius-sm, 5px)',
            boxShadow: '0 8px 24px var(--wks-shadow, rgba(0,0,0,0.4))',
            overflow: 'hidden',
          }}
        >
          <div
            style={{
              padding: 7,
              borderBottom: '1px solid var(--wks-border-subtle, var(--wks-border))',
            }}
          >
            <input
              ref={inputRef}
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={onKeyDown}
              placeholder="Search…"
              style={{ ...inputStyle, width: '100%' }}
            />
          </div>
          <div ref={listRef} style={{ maxHeight: 240, overflowY: 'auto', padding: 4 }}>
            {filtered.length === 0 ? (
              <div
                style={{
                  padding: '10px 12px',
                  fontSize: '0.75rem',
                  color: 'var(--wks-text-faint)',
                }}
              >
                No matches
              </div>
            ) : (
              filtered.map((o, i) => {
                const isSel = o.value === value;
                const isHi = i === highlight;
                const showHeader = !!o.group && (i === 0 || filtered[i - 1].group !== o.group);
                return (
                  <React.Fragment key={o.value}>
                    {showHeader && (
                      <div
                        style={{
                          padding: '6px 10px 2px',
                          fontSize: '0.62rem',
                          fontWeight: 700,
                          letterSpacing: '0.06em',
                          textTransform: 'uppercase',
                          color: 'var(--wks-text-faint)',
                        }}
                      >
                        {o.group}
                      </div>
                    )}
                    <button
                      data-opt-idx={i}
                      onMouseEnter={() => setHighlight(i)}
                      onClick={() => choose(o.value)}
                      style={{
                        display: 'flex',
                        alignItems: 'center',
                        gap: 8,
                        width: '100%',
                        padding: '6px 10px',
                        fontSize: '0.78rem',
                        fontFamily: 'inherit',
                        textAlign: 'left',
                        border: 'none',
                        borderRadius: 4,
                        cursor: 'pointer',
                        background: isHi ? 'var(--wks-bg-hover)' : 'transparent',
                        color: isSel
                          ? 'var(--wks-accent-text, var(--wks-accent))'
                          : 'var(--wks-text-secondary)',
                        fontWeight: isSel ? 600 : 400,
                      }}
                    >
                      {o.swatch && swatchDot(o.swatch)}
                      <span
                        style={{
                          flex: 1,
                          overflow: 'hidden',
                          textOverflow: 'ellipsis',
                          whiteSpace: 'nowrap',
                        }}
                      >
                        {o.label}
                      </span>
                      {isSel && <span style={{ flexShrink: 0, fontSize: '0.75rem' }}>✓</span>}
                    </button>
                  </React.Fragment>
                );
              })
            )}
          </div>
        </div>
      )}
    </div>
  );
}
