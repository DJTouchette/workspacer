import React from 'react';

/**
 * Workspacer brand marks, rendered in CSS so they inherit the active theme's
 * accent (`--wks-accent`) and mono font (`--wks-font-mono`).
 *
 *   <BrandMark />  →  { ▮ }   the code-interpolation mark (icon)
 *   <Wordmark />   →  work{spacer}   the full wordmark
 *
 * The bracketed module reads as code interpolation and always carries the
 * accent; the cursor bar between the braces is the "spacer".
 */

interface BrandMarkProps {
  /** Font-size of the braces in px; the cursor scales relative to it. */
  size?: number;
  /** Accent color for the mark. Defaults to the theme accent. */
  color?: string;
  /** Blink the cursor bar (off by default — quiet for static chrome). */
  blink?: boolean;
  style?: React.CSSProperties;
  title?: string;
}

export const BrandMark: React.FC<BrandMarkProps> = ({
  size = 22,
  color = 'var(--wks-accent)',
  blink = false,
  style,
  title,
}) => (
  <span
    aria-hidden={title ? undefined : true}
    title={title}
    style={{
      display: 'inline-flex',
      alignItems: 'center',
      justifyContent: 'center',
      fontFamily: 'var(--wks-font-mono)',
      fontWeight: 700,
      fontSize: size,
      lineHeight: 1,
      color,
      userSelect: 'none',
      ...style,
    }}
  >
    {'{'}
    <span
      style={{
        display: 'inline-block',
        width: Math.max(2, size * 0.22),
        height: size * 0.72,
        margin: `0 ${Math.max(1, size * 0.04)}px`,
        borderRadius: Math.max(1, size * 0.05),
        background: color,
        animation: blink ? 'wks-cursor-blink 1.2s steps(1) infinite' : 'none',
      }}
    />
    {'}'}
  </span>
);

interface BrandSpinnerProps {
  /** Font-size of the braces in px; the track and bar scale relative to it. */
  size?: number;
  /** Accent color for the mark. Defaults to the theme accent. */
  color?: string;
  style?: React.CSSProperties;
  title?: string;
}

/**
 * The brand mark as a loading indicator: the cursor bar shuttles back and
 * forth between the braces. Drop-in wherever a spinner would sit — reads as
 * "workspacer is working" instead of a generic throbber.
 */
export const BrandSpinner: React.FC<BrandSpinnerProps> = ({
  size = 16,
  color = 'var(--wks-accent)',
  style,
  title,
}) => {
  const barW = Math.max(2, size * 0.18);
  const trackW = Math.round(size * 0.85);
  return (
    <span
      aria-hidden={title ? undefined : true}
      title={title}
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        justifyContent: 'center',
        fontFamily: 'var(--wks-font-mono)',
        fontWeight: 700,
        fontSize: size,
        lineHeight: 1,
        color,
        userSelect: 'none',
        ...style,
      }}
    >
      {'{'}
      <span
        style={{
          display: 'inline-block',
          position: 'relative',
          width: trackW,
          height: size * 0.72,
          margin: `0 ${Math.max(1, size * 0.06)}px`,
        }}
      >
        <span
          style={{
            position: 'absolute',
            top: 0,
            left: 0,
            width: barW,
            height: '100%',
            borderRadius: Math.max(1, size * 0.05),
            background: color,
            ['--wks-brand-slide-dist' as never]: `${trackW - barW}px`,
            animation: 'wks-brand-slide 0.8s ease-in-out infinite alternate',
          }}
        />
      </span>
      {'}'}
    </span>
  );
};

interface WordmarkProps {
  /** Font-size in px. */
  size?: number;
  /** Accent color for the `{spacer}` module. Defaults to the theme accent. */
  accent?: string;
  /** Color for the leading `work`. Defaults to the primary text color. */
  textColor?: string;
  style?: React.CSSProperties;
}

export const Wordmark: React.FC<WordmarkProps> = ({
  size = 18,
  accent = 'var(--wks-accent)',
  textColor = 'var(--wks-text-primary)',
  style,
}) => (
  <span
    style={{
      fontFamily: 'var(--wks-font-mono)',
      fontWeight: 700,
      fontSize: size,
      letterSpacing: '-0.045em',
      lineHeight: 1,
      color: textColor,
      whiteSpace: 'nowrap',
      userSelect: 'none',
      ...style,
    }}
  >
    work<span style={{ color: accent }}>{'{spacer}'}</span>
  </span>
);
