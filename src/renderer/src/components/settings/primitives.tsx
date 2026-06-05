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
