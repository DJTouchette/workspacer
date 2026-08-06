import { useCallback, useEffect, useRef } from 'react';
import { useTheme } from './useTheme';
import { webviewThemeCSS, webviewThemeJS } from '../lib/webviewTheme';
import { webviewSettingsJS } from '../lib/webviewSettings';

/**
 * The two injections every plugin webview needs, wherever it's hosted.
 *
 * A plugin page is a separate document, so neither the app's `--wks-*` theme
 * tokens nor the plugin's saved settings reach it on their own. BrowserPane has
 * done this for plugin *panes* since they existed; widgets are the same kind of
 * guest in a different frame, and a widget that ignored the user's theme or
 * couldn't read its own settings would be a second-class contribution for no
 * reason. So the logic lives here and both hosts call it.
 *
 * Both injections are re-applied on every `dom-ready` (a fresh document drops
 * anything previously inserted) and whenever the source changes underneath —
 * theme on switch, settings on save — so a plugin reconfigures live without a
 * reload.
 *
 * KNOWN DUPLICATION: BrowserPane still does this inline. Its copy shares one
 * `readyRef` with a third injection (the key forwarder) whose ordering matters,
 * so lifting only these two out would leave that component with two competing
 * notions of "ready". Consolidating means moving the key forwarder too — worth
 * doing, deliberately, not as a side effect of adding widgets. Until then a
 * change to the injection contract has to land in both places.
 *
 * @param ref      the <webview> element (typed as HTMLElement — its Electron
 *                 methods, insertCSS/executeJavaScript, aren't in lib.dom, and
 *                 the Electron namespace isn't in the renderer's tsconfig)
 * @param pluginId the plugin whose settings to deliver; omit to inject theme only
 */
export function usePluginWebview(
  ref: React.RefObject<HTMLElement | null>,
  pluginId?: string,
): void {
  const { theme } = useTheme();
  const themeRef = useRef(theme);
  themeRef.current = theme;
  const readyRef = useRef(false);
  const cssKeyRef = useRef<string | null>(null);

  const applyTheme = useCallback(async (newDocument = false) => {
    const wv = ref.current as any;
    if (!wv || !readyRef.current) return;
    try {
      // insertCSS keys die with the document they were inserted into.
      if (newDocument) cssKeyRef.current = null;
      if (cssKeyRef.current && wv.removeInsertedCSS) {
        await wv.removeInsertedCSS(cssKeyRef.current).catch(() => {});
        cssKeyRef.current = null;
      }
      cssKeyRef.current = await wv.insertCSS(webviewThemeCSS(themeRef.current));
      await wv.executeJavaScript(webviewThemeJS(themeRef.current));
    } catch {
      /* mid-navigation or destroyed — the next dom-ready re-applies */
    }
  }, [ref]);

  const applySettings = useCallback(async () => {
    if (!pluginId) return;
    const wv = ref.current as any;
    if (!wv || !readyRef.current) return;
    try {
      const values = (await window.electronAPI.getPluginSettings?.(pluginId)) ?? {};
      await wv.executeJavaScript(webviewSettingsJS(values));
    } catch {
      /* mid-navigation — re-applied on next dom-ready */
    }
  }, [ref, pluginId]);

  const applyThemeRef = useRef(applyTheme);
  applyThemeRef.current = applyTheme;
  const applySettingsRef = useRef(applySettings);
  applySettingsRef.current = applySettings;

  // Re-inject when the user switches theme with the plugin on screen.
  useEffect(() => {
    void applyTheme();
  }, [theme, applyTheme]);

  // …and when the user saves this plugin's settings.
  useEffect(() => {
    if (!pluginId) return;
    const off = window.electronAPI.onPluginSettingsChanged?.((changedId: string) => {
      if (changedId === pluginId) void applySettingsRef.current();
    });
    return () => off?.();
  }, [pluginId]);

  useEffect(() => {
    const wv = ref.current as any;
    if (!wv) return;
    const onDomReady = () => {
      readyRef.current = true;
      // Theme first, so the page never flashes unstyled; then settings, so the
      // plugin configures itself before first paint.
      void applyThemeRef.current(true);
      void applySettingsRef.current();
    };
    wv.addEventListener('dom-ready', onDomReady);
    return () => {
      readyRef.current = false;
      wv.removeEventListener('dom-ready', onDomReady);
    };
  }, [ref]);
}
