import React from 'react';

export const inputStyle: React.CSSProperties = {
  height: '24px',
  padding: '0 8px',
  fontSize: '0.65rem',
  backgroundColor: 'var(--wks-bg-input)',
  color: 'var(--wks-text-secondary)',
  border: '1px solid var(--wks-border)',
  borderRadius: '3px',
  outline: 'none',
  fontFamily: 'inherit',
  boxSizing: 'border-box' as const,
};

export function SmallButton({ label, onClick, primary, danger }: { label: string; onClick: () => void; primary?: boolean; danger?: boolean }) {
  return (
    <button
      onClick={(e) => { e.stopPropagation(); onClick(); }}
      style={{
        padding: '2px 8px',
        fontSize: '0.6rem',
        fontFamily: 'inherit',
        fontWeight: 500,
        backgroundColor: primary ? 'var(--wks-accent)' : 'transparent',
        color: danger ? 'var(--wks-error)' : primary ? '#fff' : 'var(--wks-text-muted)',
        border: primary ? '1px solid var(--wks-accent)' : '1px solid var(--wks-border)',
        borderRadius: '3px',
        cursor: 'pointer',
        height: 'auto',
        lineHeight: '1.4',
        margin: 0,
        width: 'auto',
      }}
    >
      {label}
    </button>
  );
}

export function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div style={{ marginBottom: '20px' }}>
      <div style={{
        fontSize: '0.7rem',
        fontWeight: 600,
        color: 'var(--wks-text-tertiary)',
        marginBottom: '8px',
        paddingBottom: '4px',
        borderBottom: '1px solid var(--wks-border)',
      }}>
        {title}
      </div>
      <div style={{ display: 'flex', flexDirection: 'column', gap: '8px' }}>
        {children}
      </div>
    </div>
  );
}

export function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
      <span style={{ fontSize: '0.7rem', color: 'var(--wks-text-muted)' }}>{label}</span>
      {children}
    </div>
  );
}

export function CheckRow({ label, checked, onChange, disabled }: { label: string; checked: boolean; onChange: (v: boolean) => void; disabled?: boolean }) {
  return (
    <label
      style={{
        display: 'flex', alignItems: 'center', justifyContent: 'space-between',
        cursor: disabled ? 'default' : 'pointer', opacity: disabled ? 0.45 : 1,
      }}
    >
      <span style={{ fontSize: '0.7rem', color: 'var(--wks-text-muted)' }}>{label}</span>
      <input
        type="checkbox"
        checked={checked}
        disabled={disabled}
        onChange={(e) => onChange(e.target.checked)}
        style={{ accentColor: 'var(--wks-accent)', cursor: disabled ? 'default' : 'pointer' }}
      />
    </label>
  );
}

export function ModeButton({ label, active, onClick }: { label: string; active: boolean; onClick: () => void }) {
  return (
    <button
      onClick={onClick}
      style={{
        padding: '3px 12px',
        fontSize: '0.65rem',
        fontFamily: 'inherit',
        fontWeight: active ? 600 : 400,
        backgroundColor: active ? 'var(--wks-accent)' : 'var(--wks-bg-elevated)',
        color: active ? '#fff' : 'var(--wks-text-muted)',
        border: active ? '1px solid var(--wks-accent)' : '1px solid var(--wks-border)',
        borderRadius: '3px',
        cursor: 'pointer',
        height: '24px',
        lineHeight: '1',
        margin: 0,
        width: 'auto',
      }}
    >
      {label}
    </button>
  );
}

export interface SelectOption {
  value: string;
  label: string;
  /** Optional color chip shown left of the label (e.g. a theme's accent). */
  swatch?: string;
}

/**
 * A compact, searchable single-select (combobox). Click to open a popover with
 * a filter box + keyboard-navigable list. Matches the settings inline-style /
 * CSS-var conventions and stays a fixed trigger size so rows don't reflow.
 */
export function SearchableSelect({
  value,
  options,
  onChange,
  placeholder,
  width = 170,
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
    const q = query.trim().toLowerCase();
    if (!q) return options;
    return options.filter(
      (o) => o.label.toLowerCase().includes(q) || o.value.toLowerCase().includes(q),
    );
  }, [options, query]);

  // Close on outside click.
  React.useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener('mousedown', onDoc);
    return () => document.removeEventListener('mousedown', onDoc);
  }, [open]);

  // Reset + focus the filter each time we open; start the highlight on the
  // current value so Enter re-selects it.
  React.useEffect(() => {
    if (!open) return;
    setQuery('');
    const idx = options.findIndex((o) => o.value === value);
    setHighlight(idx >= 0 ? idx : 0);
    const t = setTimeout(() => inputRef.current?.focus(), 0);
    return () => clearTimeout(t);
  }, [open]); // eslint-disable-line react-hooks/exhaustive-deps

  // Keep the highlight valid as the filter narrows, and scroll it into view.
  React.useEffect(() => {
    setHighlight((h) => Math.min(h, Math.max(0, filtered.length - 1)));
  }, [filtered.length]);
  React.useEffect(() => {
    if (!open) return;
    const el = listRef.current?.children[highlight] as HTMLElement | undefined;
    el?.scrollIntoView({ block: 'nearest' });
  }, [highlight, open]);

  const choose = (v: string) => { onChange(v); setOpen(false); };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'ArrowDown') { e.preventDefault(); setHighlight((h) => Math.min(filtered.length - 1, h + 1)); }
    else if (e.key === 'ArrowUp') { e.preventDefault(); setHighlight((h) => Math.max(0, h - 1)); }
    else if (e.key === 'Enter') { e.preventDefault(); const o = filtered[highlight]; if (o) choose(o.value); }
    else if (e.key === 'Escape') { e.preventDefault(); setOpen(false); }
  };

  const swatchDot = (color: string) => (
    <span style={{
      width: 9, height: 9, borderRadius: '50%', background: color, flexShrink: 0,
      border: '1px solid rgba(255,255,255,0.15)',
    }} />
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
        <span style={{ flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', color: selected ? 'var(--wks-text-secondary)' : 'var(--wks-text-faint)' }}>
          {selected ? selected.label : (placeholder || 'Select…')}
        </span>
        <span style={{ color: 'var(--wks-text-faint)', fontSize: '0.6rem', flexShrink: 0 }}>▾</span>
      </button>

      {open && (
        <div style={{
          position: 'absolute', top: 'calc(100% + 4px)', right: 0, width: Math.max(width, 200), zIndex: 100,
          background: 'var(--wks-bg-elevated)', border: '1px solid var(--wks-border)',
          borderRadius: 'var(--wks-radius-sm, 4px)', boxShadow: '0 8px 24px var(--wks-shadow, rgba(0,0,0,0.4))',
          overflow: 'hidden',
        }}>
          <div style={{ padding: 6, borderBottom: '1px solid var(--wks-border-subtle, var(--wks-border))' }}>
            <input
              ref={inputRef}
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={onKeyDown}
              placeholder="Search…"
              style={{ ...inputStyle, width: '100%' }}
            />
          </div>
          <div ref={listRef} style={{ maxHeight: 220, overflowY: 'auto', padding: 4 }}>
            {filtered.length === 0 ? (
              <div style={{ padding: '8px 10px', fontSize: '0.65rem', color: 'var(--wks-text-faint)' }}>No matches</div>
            ) : (
              filtered.map((o, i) => {
                const isSel = o.value === value;
                const isHi = i === highlight;
                return (
                  <button
                    key={o.value}
                    onMouseEnter={() => setHighlight(i)}
                    onClick={() => choose(o.value)}
                    style={{
                      display: 'flex', alignItems: 'center', gap: 8, width: '100%',
                      padding: '5px 8px', fontSize: '0.65rem', fontFamily: 'inherit',
                      textAlign: 'left', border: 'none', borderRadius: 3, cursor: 'pointer',
                      background: isHi ? 'var(--wks-bg-hover)' : 'transparent',
                      color: isSel ? 'var(--wks-accent-text, var(--wks-accent))' : 'var(--wks-text-secondary)',
                      fontWeight: isSel ? 600 : 400,
                    }}
                  >
                    {o.swatch && swatchDot(o.swatch)}
                    <span style={{ flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{o.label}</span>
                    {isSel && <span style={{ flexShrink: 0, fontSize: '0.7rem' }}>✓</span>}
                  </button>
                );
              })
            )}
          </div>
        </div>
      )}
    </div>
  );
}
