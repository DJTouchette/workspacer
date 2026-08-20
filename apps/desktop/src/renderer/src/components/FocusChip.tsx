/**
 * FocusChip — passive focus telemetry for the command layer.
 *
 * A persistent bottom-right chip answering the one question a prefix-driven
 * keyboard mode leaves open: "where do my UNPREFIXED keys go right now?"
 *   INSERT — a composer/input (keys type text)
 *   TERM   — an xterm pane (keys go to the agent/shell)
 *   BROWSE — a webview/iframe guest (keys go to the page)
 *   APP    — nothing editable focused
 *
 * DOCTRINE (COMMAND_LAYER.md): pure telemetry. Derived exclusively from
 * focusin/focusout — it never moves focus, never suppresses anything, and
 * renders only real DOM focus state, never optimistic state. Hidden while the
 * layer is armed (the strip owns that moment) and entirely absent when the
 * layer is disabled.
 */
import React, { useEffect, useState } from 'react';

export type Face = 'insert' | 'term' | 'browse' | 'app';

const FACES: Record<Face, { label: string; hint: string; color: string }> = {
  insert: { label: 'INSERT', hint: 'keys type into the composer', color: 'var(--wks-success)' },
  term: { label: 'TERM', hint: 'keys go to the agent / shell', color: 'var(--wks-error)' },
  browse: { label: 'BROWSE', hint: 'keys go to the embedded page', color: 'var(--wks-warning)' },
  app: { label: 'APP', hint: 'no editable surface focused', color: 'var(--wks-text-faint)' },
};

function faceFor(target: EventTarget | null): Face {
  if (!(target instanceof HTMLElement)) return 'app';
  if (target.closest('.xterm')) return 'term';
  const tag = target.tagName;
  if (tag === 'WEBVIEW' || tag === 'IFRAME') return 'browse';
  if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || target.isContentEditable)
    return 'insert';
  return 'app';
}

const FocusChip: React.FC<{ enabled: boolean; armed: boolean }> = ({ enabled, armed }) => {
  const [face, setFace] = useState<Face>('app');

  useEffect(() => {
    if (!enabled) return;
    const onFocusIn = (e: FocusEvent) => setFace(faceFor(e.target));
    const onFocusOut = () => setFace('app');
    // Seed from wherever focus already sits (the chip may mount mid-session).
    setFace(faceFor(document.activeElement));
    window.addEventListener('focusin', onFocusIn);
    window.addEventListener('focusout', onFocusOut);
    return () => {
      window.removeEventListener('focusin', onFocusIn);
      window.removeEventListener('focusout', onFocusOut);
    };
  }, [enabled]);

  if (!enabled || armed) return null;
  return <FocusChipView face={face} />;
};

/** Presentational chip — split out so the design harness can render every
 *  face side by side without faking focus events. */
export const FocusChipView: React.FC<{ face: Face }> = ({ face }) => {
  const f = FACES[face];
  return (
    <div
      role="status"
      aria-live="polite"
      title={f.hint}
      style={{
        position: 'fixed',
        bottom: 16,
        right: 16,
        zIndex: 190, // under the chord chrome (200) — armed hides it anyway
        display: 'flex',
        alignItems: 'center',
        gap: 6,
        padding: '2px 9px',
        fontFamily: 'var(--wks-font-mono)',
        fontSize: '0.62rem',
        fontWeight: 700,
        letterSpacing: '0.08em',
        color: f.color,
        backgroundColor: 'var(--wks-glass-strong)',
        backdropFilter: 'blur(var(--wks-glass-blur)) saturate(170%)',
        WebkitBackdropFilter: 'blur(var(--wks-glass-blur)) saturate(170%)',
        border: '1px solid var(--wks-glass-border)',
        borderRadius: 'var(--wks-radius-md)',
      }}
    >
      <span
        aria-hidden
        style={{
          width: 6,
          height: 6,
          borderRadius: '50%',
          backgroundColor: f.color,
        }}
      />
      {f.label}
    </div>
  );
};

export default FocusChip;
