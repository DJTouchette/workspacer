import React from 'react';
import { BrandMark, Wordmark } from './Brand';
import { formatBinding } from '../lib/shortcuts';

/**
 * The welcome card. Two showings, both as a modal overlay:
 *
 * - First run: auto-opens once the config + session restore settle, when there
 *   are no real agent workspaces yet and the user hasn't dismissed it. Orients
 *   a brand-new user (spawn, palette, inbox/fleet, settings) using their
 *   *actual* configured shortcuts; dismissing persists onboardingDismissed.
 * - Replay: the "Show Welcome" palette command re-opens the same card anytime.
 */
const Onboarding: React.FC<{
  onSpawn: () => void;
  onDismiss: () => void;
  /** Resolved keybinding combos (config.keybindings.shortcuts). */
  shortcuts: Record<string, string>;
  /** Configured chord prefix, so "prefix i"-style bindings render correctly. */
  prefix?: string;
  /** Render as a modal overlay instead of filling the content area. */
  overlay?: boolean;
  /** First-run showing (dismiss persists the flag) vs a palette replay. */
  firstRun?: boolean;
}> = ({ onSpawn, onDismiss, shortcuts, prefix, overlay, firstRun }) => {
  // Fallbacks mirror configDefaults.ts; the map is normally already merged
  // with defaults, so these only cover a not-yet-loaded config.
  const k = (id: string, fallback: string) => formatBinding(shortcuts[id] || fallback, prefix);

  const rows: Array<{ combo: string; label: string }> = [
    {
      combo: k('command-palette', 'ctrl+shift+p'),
      label: 'Command palette — every action, searchable',
    },
    {
      combo: k('toggle-inbox', 'ctrl+shift+i'),
      label: 'Triage Inbox — approvals & questions across agents',
    },
    { combo: k('toggle-fleet', 'ctrl+shift+f'), label: 'Fleet Deck — a live radar of every agent' },
    { combo: k('settings', 'ctrl+,'), label: 'Settings — themes, keybindings, and more' },
    { combo: k('toggle-help', 'f1'), label: 'Keyboard shortcuts — the full list, anytime' },
  ];

  const card = (
    <div
      onClick={(e) => e.stopPropagation()}
      style={{
        width: 'min(560px, 100%)',
        background: 'var(--wks-bg-surface)',
        border: '1px solid var(--wks-glass-border)',
        borderRadius: 'var(--wks-radius-lg)',
        padding: '26px 26px 22px',
        boxShadow: '0 12px 40px var(--wks-shadow)',
        animation: 'claudeFadeIn 0.2s ease-out',
      }}
    >
      {/* Brand lockup — the { ▮ } mark + work{spacer} wordmark. */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 18 }}>
        <span
          style={{
            width: 44,
            height: 44,
            flexShrink: 0,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            background: 'var(--wks-bg-base)',
            border: '1px solid var(--wks-border-subtle)',
            borderRadius: 'var(--wks-radius-lg)',
          }}
        >
          <BrandMark size={24} blink />
        </span>
        <Wordmark size={24} />
      </div>

      <div
        style={{
          fontSize: '1.25rem',
          fontWeight: 700,
          letterSpacing: '-0.01em',
          color: 'var(--wks-text-primary)',
        }}
      >
        Welcome to Workspacer
      </div>
      <div
        style={{
          fontSize: '0.85rem',
          color: 'var(--wks-text-secondary)',
          marginTop: 6,
          lineHeight: 1.5,
        }}
      >
        A cockpit for running many Claude Code agents side by side. Each agent is a long-lived
        session with its own tabs &amp; panes — it keeps running until you terminate it.
      </div>

      <button
        onClick={onSpawn}
        style={{
          marginTop: 18,
          display: 'inline-flex',
          alignItems: 'center',
          gap: 10,
          fontSize: '0.85rem',
          fontFamily: 'inherit',
          fontWeight: 700,
          cursor: 'pointer',
          background: 'var(--wks-accent)',
          color: 'var(--wks-text-on-accent, #fff)',
          border: 'none',
          borderRadius: 6,
          padding: '10px 18px',
        }}
      >
        + Spawn your first agent
        <span
          style={{
            fontSize: '0.66rem',
            fontFamily: 'var(--wks-font-mono)',
            fontWeight: 600,
            opacity: 0.75,
          }}
        >
          {k('spawn-agent', 'ctrl+shift+n')}
        </span>
      </button>

      <div style={{ marginTop: 20, display: 'flex', flexDirection: 'column', gap: 9 }}>
        {rows.map((r) => (
          <div
            key={r.label}
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 10,
              fontSize: '0.8rem',
              color: 'var(--wks-text-secondary)',
            }}
          >
            <kbd
              style={{
                flexShrink: 0,
                minWidth: 84,
                textAlign: 'center',
                fontSize: '0.68rem',
                fontFamily: 'var(--wks-font-mono)',
                color: 'var(--wks-text-primary)',
                border: '1px solid var(--wks-glass-border)',
                borderRadius: 4,
                padding: '2px 6px',
                background: 'var(--wks-bg-base)',
              }}
            >
              {r.combo}
            </kbd>
            <span>{r.label}</span>
          </div>
        ))}
      </div>

      <div style={{ marginTop: 22, display: 'flex', justifyContent: 'flex-end' }}>
        <button
          onClick={onDismiss}
          style={{
            fontSize: '0.76rem',
            fontFamily: 'inherit',
            fontWeight: 600,
            cursor: 'pointer',
            background: 'transparent',
            color: 'var(--wks-text-faint)',
            border: '1px solid var(--wks-glass-border)',
            borderRadius: 6,
            padding: '7px 14px',
          }}
        >
          {firstRun ? "Got it — don't show again" : 'Close'}
        </button>
      </div>
    </div>
  );

  if (overlay) {
    return (
      <div
        role="dialog"
        aria-label="Welcome"
        onClick={onDismiss}
        style={{
          position: 'fixed',
          top: 0,
          left: 0,
          right: 0,
          bottom: 0,
          backgroundColor: 'var(--wks-overlay)',
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          padding: 24,
          boxSizing: 'border-box',
          zIndex: 1000,
        }}
      >
        {card}
      </div>
    );
  }

  return (
    <div
      role="region"
      aria-label="Welcome"
      style={{
        height: '100%',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        padding: 24,
        boxSizing: 'border-box',
      }}
    >
      {card}
    </div>
  );
};

export default Onboarding;
