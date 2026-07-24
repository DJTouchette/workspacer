/**
 * The UI font choice (config.ui.fontFamily) resolved into --wks-font-sans.
 * Every sans surface reads the token (body inherits it, chrome references it),
 * so switching fonts is one CSS-variable write — no remounts.
 *
 * Accepted values:
 *  - a bundled id: 'hanken' | 'inter' | 'space-grotesk' | 'manrope' | 'outfit'
 *    (variable woff2s shipped in assets/fonts, declared in App.css)
 *  - 'custom:<Family Name>' — a user-uploaded font file installed under
 *    ~/.workspacer/fonts; the main process @font-face-injects every installed
 *    file at boot (and on upload), so the family is loadable by name
 *  - anything else — including the pre-feature "Inter, system-ui, sans-serif"
 *    stack that older configs persisted without any consumer — falls back to
 *    the default so nobody's UI changes out from under them.
 */

export interface UiFontChoice {
  id: string;
  label: string;
  /** Quoted css font-family name. */
  family: string;
}

export const UI_FONTS: UiFontChoice[] = [
  { id: 'hanken', label: 'Hanken Grotesk', family: "'Hanken Grotesk'" },
  { id: 'inter', label: 'Inter', family: "'Inter'" },
  { id: 'space-grotesk', label: 'Space Grotesk', family: "'Space Grotesk'" },
  { id: 'manrope', label: 'Manrope', family: "'Manrope'" },
  { id: 'outfit', label: 'Outfit', family: "'Outfit'" },
];

export const DEFAULT_UI_FONT_ID = 'hanken';
export const CUSTOM_FONT_PREFIX = 'custom:';

const FALLBACK_STACK =
  "-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, 'Helvetica Neue', sans-serif";

/** The full font-family stack for a config value (see module docs). */
export function resolveUiFontStack(value: string | undefined): string {
  const v = (value ?? '').trim();
  if (v.startsWith(CUSTOM_FONT_PREFIX)) {
    const family = v.slice(CUSTOM_FONT_PREFIX.length).trim().replace(/['"]/g, '');
    if (family) return `'${family}', 'Hanken Grotesk', ${FALLBACK_STACK}`;
  }
  const chosen = UI_FONTS.find((f) => f.id === v) ?? UI_FONTS[0];
  return `${chosen.family}, ${FALLBACK_STACK}`;
}

/** Write the choice into --wks-font-sans (inline on <html>, so it wins over
 *  the App.css :root first-paint default). */
export function applyUiFont(value: string | undefined): void {
  document.documentElement.style.setProperty('--wks-font-sans', resolveUiFontStack(value));
}
