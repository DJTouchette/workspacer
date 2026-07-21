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

export function webviewThemeCSS(theme: Theme): string {
  const vars = Object.entries(cssVarsOf(theme))
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
