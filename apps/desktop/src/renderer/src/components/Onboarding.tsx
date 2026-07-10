import React from 'react';
import { BrandMark, Wordmark } from './Brand';
import { ClaudeLogo, OpenAILogo, OpenCodeLogo, PiLogo } from './agentLogos';
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

/** One shortcut combo as individual keycaps: "Ctrl+Shift+P" → [Ctrl][Shift][P].
 *  Chord bindings ("Ctrl+Space T W") flow into extra caps in press order. */
const Keys: React.FC<{ combo: string; onAccent?: boolean }> = ({ combo, onAccent }) => (
  <span style={{ display: 'inline-flex', alignItems: 'center', gap: 4 }}>
    {combo
      .split(/[+\s]+/)
      .filter(Boolean)
      .map((key, i) => (
        <kbd
          key={i}
          style={{
            fontFamily: 'var(--wks-font-mono)',
            fontSize: '0.62rem',
            fontWeight: 600,
            lineHeight: 1,
            padding: '4px 6px',
            borderRadius: 4,
            whiteSpace: 'nowrap',
            ...(onAccent
              ? {
                  color: 'var(--wks-text-on-accent, #fff)',
                  background: 'rgba(255, 255, 255, 0.16)',
                  border: '1px solid rgba(255, 255, 255, 0.28)',
                }
              : {
                  color: 'var(--wks-text-secondary)',
                  background: 'var(--wks-bg-elevated)',
                  border: '1px solid var(--wks-border)',
                  boxShadow: 'inset 0 -1.5px 0 var(--wks-border-subtle)',
                }),
          }}
        >
          {key}
        </kbd>
      ))}
  </span>
);

const Onboarding: React.FC<{
  onSpawn: () => void;
  onDismiss: () => void;
  /** Open Settings on the Keybindings section ("customize your keybinds"). */
  onOpenKeybindings?: () => void;
  /** Resolved keybinding combos (config.keybindings.shortcuts). */
  shortcuts: Record<string, string>;
  /** Configured chord prefix, so "prefix i"-style bindings render correctly. */
  prefix?: string;
  /** Render as a modal overlay instead of filling the content area. */
  overlay?: boolean;
  /** First-run showing (dismiss persists the flag) vs a palette replay. */
  firstRun?: boolean;
}> = ({ onSpawn, onDismiss, onOpenKeybindings, shortcuts, prefix, overlay, firstRun }) => {
  // Fallbacks mirror configDefaults.ts; the map is normally already merged
  // with defaults, so these only cover a not-yet-loaded config.
  const k = (id: string, fallback: string) => formatBinding(shortcuts[id] || fallback, prefix);

  const rows: Array<{ combo: string; title: string; desc: string }> = [
    {
      combo: k('command-palette', 'ctrl+shift+p'),
      title: 'Command palette',
      desc: 'every action, searchable',
    },
    {
      combo: k('toggle-inbox', 'ctrl+shift+i'),
      title: 'Triage Inbox',
      desc: 'approvals & questions across agents',
    },
    {
      combo: k('toggle-fleet', 'ctrl+shift+f'),
      title: 'Fleet Deck',
      desc: 'a live radar of every agent',
    },
    { combo: k('settings', 'ctrl+,'), title: 'Settings', desc: 'themes, keybindings, and more' },
    {
      combo: k('toggle-help', 'f1'),
      title: 'Keyboard shortcuts',
      desc: 'the full list, anytime',
    },
  ];

  const card = (
    <div
      className="wks-welcome-card"
      onClick={(e) => e.stopPropagation()}
      style={{
        position: 'relative',
        width: 'min(600px, 100%)',
        maxHeight: 'min(760px, 92vh)',
        overflowY: 'auto',
        borderRadius: 'var(--wks-radius-lg)',
      }}
    >
      {/* Soft accent glow washing down from behind the brand — pure decoration. */}
      <div
        aria-hidden
        style={{
          position: 'absolute',
          top: -180,
          left: '50%',
          transform: 'translateX(-50%)',
          width: 520,
          height: 380,
          borderRadius: '50%',
          background:
            'radial-gradient(circle, color-mix(in srgb, var(--wks-accent) 14%, transparent) 0%, transparent 68%)',
          pointerEvents: 'none',
        }}
      />

      {/* Hero — brand lockup, greeting, spawn CTA. */}
      <div
        style={{
          position: 'relative',
          display: 'flex',
          flexDirection: 'column',
          alignItems: 'center',
          textAlign: 'center',
          padding: '38px 32px 30px',
        }}
      >
        <BrandMark size={46} blink />
        <div style={{ marginTop: 18 }}>
          <span
            style={{
              fontSize: '1.35rem',
              fontWeight: 700,
              letterSpacing: '-0.015em',
              color: 'var(--wks-text-primary)',
            }}
          >
            Welcome to{' '}
          </span>
          <Wordmark size={21} style={{ verticalAlign: 'baseline' }} />
        </div>
        <div
          style={{
            marginTop: 10,
            maxWidth: 400,
            fontSize: '0.84rem',
            color: 'var(--wks-text-secondary)',
            lineHeight: 1.55,
          }}
        >
          A cockpit for running many coding agents side by side. Each agent is a long-lived session
          with its own tabs &amp; panes — it keeps working until you terminate it.
        </div>

        <button
          className="wks-welcome-cta"
          onClick={onSpawn}
          style={{
            marginTop: 24,
            display: 'inline-flex',
            alignItems: 'center',
            gap: 10,
            fontSize: '0.86rem',
            fontFamily: 'inherit',
            fontWeight: 700,
            cursor: 'pointer',
            background: 'var(--wks-accent)',
            color: 'var(--wks-text-on-accent, #fff)',
            border: 'none',
            borderRadius: 'var(--wks-radius-md, 8px)',
            padding: '11px 22px',
          }}
        >
          + Spawn your first agent
          <Keys combo={k('spawn-agent', 'ctrl+shift+n')} onAccent />
        </button>
      </div>

      {/* Get around — the five doors, with the user's real bindings. */}
      <div
        style={{
          position: 'relative',
          borderTop: '1px solid var(--wks-border-subtle)',
          padding: '18px 24px 14px',
        }}
      >
        <div
          style={{
            fontSize: '0.62rem',
            fontWeight: 700,
            letterSpacing: '0.12em',
            textTransform: 'uppercase',
            color: 'var(--wks-text-faint)',
            padding: '0 8px 8px',
          }}
        >
          Get around
        </div>
        {rows.map((r, i) => (
          <div
            key={r.title}
            className="wks-welcome-row wks-welcome-stagger"
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 12,
              padding: '7px 8px',
              animationDelay: `${0.12 + i * 0.05}s`,
            }}
          >
            <span
              style={{
                fontSize: '0.8rem',
                fontWeight: 600,
                color: 'var(--wks-text-primary)',
                whiteSpace: 'nowrap',
              }}
            >
              {r.title}
            </span>
            <span
              style={{
                flex: 1,
                fontSize: '0.76rem',
                color: 'var(--wks-text-muted)',
                overflow: 'hidden',
                textOverflow: 'ellipsis',
                whiteSpace: 'nowrap',
              }}
            >
              {r.desc}
            </span>
            <Keys combo={r.combo} />
          </div>
        ))}
        <div
          className="wks-welcome-stagger"
          style={{
            padding: '10px 8px 4px',
            fontSize: '0.72rem',
            color: 'var(--wks-text-faint)',
            animationDelay: `${0.12 + rows.length * 0.05}s`,
          }}
        >
          Not your muscle memory?{' '}
          {onOpenKeybindings ? (
            <button
              onClick={onOpenKeybindings}
              style={{
                font: 'inherit',
                fontWeight: 600,
                color: 'var(--wks-accent-text, var(--wks-accent))',
                background: 'none',
                border: 'none',
                padding: 0,
                cursor: 'pointer',
                textDecoration: 'underline',
                textUnderlineOffset: 3,
              }}
            >
              Rebind everything in Settings → Keybindings
            </button>
          ) : (
            <span>rebind everything in Settings → Keybindings</span>
          )}
          .
        </div>
      </div>

      {/* Footer — quiet provider strip + dismiss. */}
      <div
        style={{
          position: 'relative',
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          gap: 12,
          borderTop: '1px solid var(--wks-border-subtle)',
          padding: '14px 24px 16px',
        }}
      >
        <div
          title="claude · codex · opencode · pi"
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 14,
            opacity: 0.4,
            color: 'var(--wks-text-muted)',
          }}
        >
          <ClaudeLogo size={14} />
          <OpenAILogo size={14} />
          <OpenCodeLogo size={14} />
          <PiLogo size={14} />
        </div>
        <button
          className="wks-welcome-dismiss"
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
        className="wks-welcome-backdrop"
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
