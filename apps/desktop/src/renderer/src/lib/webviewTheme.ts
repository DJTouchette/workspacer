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
 *
 * CSS_BREAKOUT alone is NOT enough: a value that stays inside the `:root{}`
 * declaration but names an image-fetching function still fires an outbound
 * request when a `--wks-*` token is consumed as a background (the `:where(html,
 * body){ background: var(--wks-bg-base) }` rule below does exactly that). E.g.
 * `url(https://evil/beacon.png)` or `image-set("https://evil/x" 1x)` contain no
 * breakout character, so they would survive and beacon on guest render — the
 * "url() beacon" this comment already named as in-scope. CSS_FETCH drops any
 * value naming url()/image()/image-set()/cross-fade() (case-insensitive, and
 * vendor-prefixed forms via substring); a legitimate color/length/box-shadow
 * value contains none of them, while color functions (rgb/rgba/hsl/var/calc)
 * are untouched. Escape tricks like `\75rl(` are already caught by the
 * backslash in CSS_BREAKOUT.
 */
const CSS_BREAKOUT = /[{}<>;@\\]|\/\*|\*\//;
const CSS_FETCH = /(?:url|image|image-set|cross-fade)\s*\(/i;
export function isSafeThemeTokenValue(value: string): boolean {
  return typeof value === 'string' && !CSS_BREAKOUT.test(value) && !CSS_FETCH.test(value);
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
