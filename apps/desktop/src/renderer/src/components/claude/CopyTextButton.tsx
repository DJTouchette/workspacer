import React, { useCallback, useEffect, useRef, useState } from 'react';
import { Check, Copy } from 'lucide-react';

/**
 * Copy-to-clipboard affordance for a block of text, revealed on hover by the
 * `.wks-hover-actions` row it sits in (see App.css). Flips to a check for a
 * beat after a successful copy — the only feedback there's room for.
 */
export const CopyTextButton: React.FC<{
  text: string;
  /** Tooltip / a11y label; the copied state appends its own. */
  label?: string;
}> = ({ text, label = 'Copy message' }) => {
  const [copied, setCopied] = useState(false);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(
    () => () => {
      if (timerRef.current) clearTimeout(timerRef.current);
    },
    [],
  );

  const copy = useCallback(async () => {
    if (!text) return;
    try {
      await navigator.clipboard?.writeText(text);
    } catch {
      // Clipboard denied (or unavailable in a non-secure context): no feedback
      // is better than a stuck "copied" tick that lied.
      return;
    }
    setCopied(true);
    if (timerRef.current) clearTimeout(timerRef.current);
    timerRef.current = setTimeout(() => setCopied(false), 1400);
  }, [text]);

  return (
    <button
      type="button"
      className="wks-hover-action"
      onClick={() => void copy()}
      title={copied ? 'Copied' : label}
      aria-label={copied ? 'Copied' : label}
    >
      {copied ? <Check size={12} strokeWidth={2} /> : <Copy size={12} strokeWidth={2} />}
      <span>{copied ? 'Copied' : 'Copy'}</span>
    </button>
  );
};
