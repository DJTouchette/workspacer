import React from 'react';

/**
 * Consistent centered states for panes and lists: a quiet "Loading…" while
 * data is in flight and a friendly empty state once it has arrived empty.
 * Use these instead of rendering nothing (looks broken) or reusing the empty
 * copy for the loading case (lies about what's happening).
 */

const wrap: React.CSSProperties = {
  width: '100%',
  height: '100%',
  minHeight: 80,
  display: 'flex',
  flexDirection: 'column',
  alignItems: 'center',
  justifyContent: 'center',
  gap: 8,
  padding: 24,
  textAlign: 'center',
  color: 'var(--wks-text-muted)',
};

export function LoadingState({ label = 'Loading…' }: { label?: string }) {
  return (
    <div style={{ ...wrap, fontSize: '0.8rem' }} aria-busy="true">
      {label}
    </div>
  );
}

export function EmptyState({
  title,
  hint,
  icon,
  action,
}: {
  title: string;
  hint?: React.ReactNode;
  icon?: React.ReactNode;
  action?: React.ReactNode;
}) {
  return (
    <div style={wrap}>
      {icon && <span style={{ color: 'var(--wks-text-faint)' }}>{icon}</span>}
      <div style={{ fontSize: '0.9rem', fontWeight: 600, color: 'var(--wks-text-secondary)' }}>{title}</div>
      {hint && (
        <div style={{ fontSize: '0.78rem', maxWidth: 360, lineHeight: 1.5 }}>{hint}</div>
      )}
      {action && <div style={{ marginTop: 4 }}>{action}</div>}
    </div>
  );
}
