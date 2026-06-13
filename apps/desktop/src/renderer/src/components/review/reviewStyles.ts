/** Injected-once CSS for the review pane: hover affordances, focus rings,
 * skeleton shimmer and scrollbars that inline styles can't express. Same
 * pattern as claude-shared's ensureKeyframes. */

const STYLE_ID = 'wks-review-styles';

export function ensureReviewStyles(): void {
  if (typeof document === 'undefined') return;
  if (document.getElementById(STYLE_ID)) return;
  const style = document.createElement('style');
  style.id = STYLE_ID;
  style.textContent = `
.wks-review-row { transition: background 80ms ease, border-color 80ms ease; }
.wks-review-row .wks-review-action { opacity: 0; transition: opacity 80ms ease; }
.wks-review-row:hover .wks-review-action { opacity: 1; }
.wks-review-row:hover { background: var(--wks-bg-hover); }

.wks-review-dir { transition: background 80ms ease; }
.wks-review-dir:hover { background: var(--wks-bg-hover); }
.wks-review-dir .wks-review-chevron { transition: transform 120ms ease; }

.wks-icon-btn {
  display: inline-flex; align-items: center; justify-content: center;
  width: 24px; height: 24px; border-radius: 6px; border: none;
  background: transparent; color: var(--wks-text-faint); cursor: pointer;
  transition: background 80ms ease, color 80ms ease;
}
.wks-icon-btn:hover:not(:disabled) { background: var(--wks-bg-hover); color: var(--wks-text-primary); }
.wks-icon-btn:disabled { opacity: 0.45; cursor: default; }

.wks-review-commit-input {
  transition: border-color 100ms ease, box-shadow 100ms ease;
}
.wks-review-commit-input:focus {
  outline: none;
  border-color: var(--wks-accent-text) !important;
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--wks-accent-text) 22%, transparent);
}

.wks-review-commit-btn { transition: opacity 100ms ease, transform 80ms ease; }
.wks-review-commit-btn:not(:disabled):hover { opacity: 0.9; }
.wks-review-commit-btn:not(:disabled):active { transform: scale(0.985); }

@keyframes wks-review-shimmer {
  0% { background-position: -300px 0; }
  100% { background-position: 300px 0; }
}
.wks-review-skeleton {
  background: linear-gradient(90deg,
    var(--wks-bg-hover) 25%,
    color-mix(in srgb, var(--wks-text-faint) 14%, var(--wks-bg-hover)) 50%,
    var(--wks-bg-hover) 75%);
  background-size: 600px 100%;
  animation: wks-review-shimmer 1.4s ease-in-out infinite;
  border-radius: 4px;
}

@keyframes wks-review-spin { to { transform: rotate(360deg); } }
.wks-review-spin { animation: wks-review-spin 0.9s linear infinite; }

.wks-review-scroll::-webkit-scrollbar { width: 10px; height: 10px; }
.wks-review-scroll::-webkit-scrollbar-thumb {
  background: color-mix(in srgb, var(--wks-text-faint) 28%, transparent);
  border-radius: 5px; border: 2px solid transparent; background-clip: padding-box;
}
.wks-review-scroll::-webkit-scrollbar-thumb:hover {
  background: color-mix(in srgb, var(--wks-text-faint) 45%, transparent);
  border: 2px solid transparent; background-clip: padding-box;
}
.wks-review-scroll::-webkit-scrollbar-corner { background: transparent; }
`;
  document.head.appendChild(style);
}
