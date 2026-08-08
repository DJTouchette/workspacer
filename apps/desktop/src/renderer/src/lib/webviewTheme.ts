/**
 * Theme bridge for plugin webviews. A <webview> is a separate document, so
 * the app's --wks-* custom properties don't reach it on their own. For
 * appMode (plugin) webviews we inject:
 *
 *  1. CSS — every --wks-* token on :root, `color-scheme` so native controls
 *     match, and zero-specificity `:where()` body defaults. The :where()
 *     wrapper means any rule the plugin writes itself (specificity ≥ 0-0-1)
 *     wins, so theme-aware plugins are never overridden — but a plugin with
 *     no styling at all still gets matching background/text.
 *
 *  2. JS — `window.__WKS_THEME__ = { name, vars }` plus a `wks-theme`
 *     CustomEvent on every change, so canvas/chart plugins can re-render.
 *     `document.documentElement.dataset.wksTheme` carries the theme name
 *     for CSS hooks like `[data-wks-theme="light"]`.
 *
 * Plugin contract (documented in hub/docs/plugin-theming.md):
 *   color: var(--wks-text-primary);
 *   window.addEventListener('wks-theme', (e) => render(e.detail.vars));
 */
import type { Theme } from '../themes';
import { cssVarsOf, isLightTheme } from '../themes';

/**
 * A theme token value is safe to concatenate into a `prop: value;` declaration
 * only if it cannot terminate that declaration or open a new selector/rule.
 *
 * The other injection path — `applyTheme` — hands each value to
 * `CSSStyleDeclaration.setProperty`, which is browser-validated and silently
 * drops a value that breaks out. This function builds a raw CSS *string* for
 * `<webview>.insertCSS`, which has no such validation, so it must do that check
 * itself. A custom theme's colors come from `config.ui.customThemes`, which a
 * non-host bus caller (plugin / remote / web / MCP facade) can write; without
 * this a color like `#fff; } input[type=password] { … }` closes `:root{}` and
 * injects an attacker rule into the guest document — including a third-party
 * origin loaded in a BrowserPane (credential-selector exfil, url() beacon).
 *
 * Every legitimate token value is a hex/rgb(a) color, a `NNpx` length, or a
 * box-shadow list — none of which contain any of these characters, so a value
 * that does is not a color the theme editor could have produced and is dropped.
 */
const CSS_BREAKOUT = /[{}<>;@\\]|\/\*|\*\//;
export function isSafeThemeTokenValue(value: string): boolean {
  return typeof value === 'string' && !CSS_BREAKOUT.test(value);
}

export function webviewThemeCSS(theme: Theme): string {
  const vars = Object.entries(cssVarsOf(theme))
    // Drop any value that could break out of the :root{} declaration block —
    // insertCSS does not validate the string the way setProperty does.
    .filter(([, value]) => isSafeThemeTokenValue(value))
    .map(([prop, value]) => `  ${prop}: ${value};`)
    .join('\n');
  return [
    ':root {',
    vars,
    `  color-scheme: ${isLightTheme(theme) ? 'light' : 'dark'};`,
    '}',
    // Zero-specificity defaults — any plugin-authored rule beats these.
    ':where(html, body) {',
    '  background: var(--wks-bg-base);',
    '  color: var(--wks-text-primary);',
    '}',
    ':where(body) {',
    "  font-family: system-ui, -apple-system, 'Segoe UI', sans-serif;",
    '}',
  ].join('\n');
}

export function webviewThemeJS(theme: Theme): string {
  const payload = JSON.stringify({ name: theme.name, vars: cssVarsOf(theme) });
  // Runs in the guest page. Idempotent; re-dispatches on every theme change.
  return `
    (() => {
      window.__WKS_THEME__ = ${payload};
      document.documentElement.dataset.wksTheme = window.__WKS_THEME__.name;
      window.dispatchEvent(new CustomEvent('wks-theme', { detail: window.__WKS_THEME__ }));
    })();
  `;
}
