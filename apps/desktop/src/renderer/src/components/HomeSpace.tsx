import React from 'react';
import { BrandMark, Wordmark } from './Brand';
import { ClaudeLogo, OpenAILogo, OpenCodeLogo, PiLogo } from './agentLogos';

/**
 * The "nothing selected" home screen — brand mark front and center over a soft
 * accent glow, with the spawn CTA and its shortcut. Replaces the plain
 * EmptyState so an empty workspace still feels like workspacer.
 */

interface HomeSpaceProps {
  onSpawn: () => void;
  /** Display form of the spawn shortcut, e.g. 'ctrl+shift+n'. */
  spawnShortcut?: string;
}

const kbd: React.CSSProperties = {
  fontFamily: 'var(--wks-font-mono)',
  fontSize: '0.68rem',
  padding: '2px 6px',
  borderRadius: 4,
  border: '1px solid var(--wks-border)',
  background: 'var(--wks-bg-elevated)',
  color: 'var(--wks-text-tertiary)',
};

export const HomeSpace: React.FC<HomeSpaceProps> = ({ onSpawn, spawnShortcut }) => (
  <div
    style={{
      position: 'relative',
      height: '100%',
      display: 'flex',
      flexDirection: 'column',
      alignItems: 'center',
      justifyContent: 'center',
      gap: 0,
      overflow: 'hidden',
      animation: 'wks-fade-in 0.35s ease-out',
    }}
  >
    {/* Soft accent glow behind the mark — pure decoration */}
    <div
      aria-hidden
      style={{
        position: 'absolute',
        width: 560,
        height: 560,
        borderRadius: '50%',
        background:
          'radial-gradient(circle, color-mix(in srgb, var(--wks-accent) 9%, transparent) 0%, transparent 65%)',
        pointerEvents: 'none',
      }}
    />

    <BrandMark size={84} blink />
    <div style={{ marginTop: 22 }}>
      <Wordmark size={26} />
    </div>
    <div
      style={{
        marginTop: 10,
        fontSize: '0.78rem',
        color: 'var(--wks-text-muted)',
        letterSpacing: '0.01em',
      }}
    >
      Your agent fleet lives here.
    </div>

    <button
      onClick={onSpawn}
      style={{
        marginTop: 30,
        fontSize: '0.8rem',
        fontFamily: 'inherit',
        fontWeight: 600,
        cursor: 'pointer',
        background: 'var(--wks-accent)',
        color: 'var(--wks-text-on-accent, #fff)',
        border: 'none',
        borderRadius: 'var(--wks-radius-md, 6px)',
        padding: '9px 20px',
        display: 'inline-flex',
        alignItems: 'center',
        gap: 8,
        zIndex: 1,
      }}
    >
      + Spawn agent
    </button>
    {spawnShortcut && (
      <div
        style={{
          marginTop: 12,
          display: 'flex',
          alignItems: 'center',
          gap: 6,
          fontSize: '0.7rem',
          color: 'var(--wks-text-faint)',
        }}
      >
        {spawnShortcut.split('+').map((k, i, arr) => (
          <React.Fragment key={i}>
            <span style={kbd}>{k}</span>
            {i < arr.length - 1 && <span>+</span>}
          </React.Fragment>
        ))}
      </div>
    )}

    {/* Providers strip — quiet nod to what the space can run */}
    <div
      style={{
        position: 'absolute',
        bottom: 28,
        display: 'flex',
        alignItems: 'center',
        gap: 18,
        opacity: 0.4,
        color: 'var(--wks-text-muted)',
      }}
      title="claude · codex · opencode · pi"
    >
      <ClaudeLogo size={15} />
      <OpenAILogo size={15} />
      <OpenCodeLogo size={15} />
      <PiLogo size={15} />
    </div>
  </div>
);

export default HomeSpace;
