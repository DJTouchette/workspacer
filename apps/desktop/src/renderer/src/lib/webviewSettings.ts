/**
 * Settings bridge for plugin webviews — the sibling of webviewTheme. A plugin
 * declares its settings in its manifest; the host persists the values and injects
 * the current set into the plugin's webview as `window.__WKS_SETTINGS__`, plus a
 * `wks-settings` CustomEvent on every change, so the plugin applies them live.
 *
 * Plugin contract:
 *   const s = window.__WKS_SETTINGS__ || {};
 *   window.addEventListener('wks-settings', (e) => apply(e.detail));
 *
 * Values are the host-merged set: the plugin's manifest defaults with the user's
 * saved overrides applied on top, so every declared setting with a default is
 * present. A plugin can still fall back to its own defaults for any key not in
 * the map (e.g. settings declared without a default).
 */
export function webviewSettingsJS(values: Record<string, unknown>): string {
  const payload = JSON.stringify(values || {});
  return `
    (() => {
      window.__WKS_SETTINGS__ = ${payload};
      window.dispatchEvent(new CustomEvent('wks-settings', { detail: window.__WKS_SETTINGS__ }));
    })();
  `;
}
