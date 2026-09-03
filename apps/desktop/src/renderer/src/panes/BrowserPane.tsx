import React, { useState, useCallback, useRef, useEffect, useMemo } from 'react';
import { AlertTriangle, X } from 'lucide-react';
import { useConfig, DEFAULT_CONFIG } from '../hooks/useConfig';
import { useTheme } from '../hooks/useTheme';
import { webviewThemeCSS, webviewThemeJS } from '../lib/webviewTheme';
import { webviewSettingsJS } from '../lib/webviewSettings';
import { resolveLeader, resolveMod } from '../lib/shortcuts';
import { isLayerArmed } from '../lib/layerArmed';
import { guestHost, guestFramePolicy } from '../lib/guestFrame';
import GuestFrame from '../components/GuestFrame';
import { sameUrl } from '../lib/browserBus';
import { requestMarkdownPreview } from '../lib/previewBus';

interface BrowserPaneProps {
  paneId: string;
  title: string;
  isActive: boolean;
  initialUrl?: string;
  appMode?: boolean;
  hibernated?: boolean;
  onUrlChange?: (url: string) => void;
  /** Plugin panes: the contributing plugin's id, used to inject its settings. */
  pluginId?: string;
}

interface Bookmark {
  name: string;
  url: string;
}

/** Why back/forward are dead in a browser — shown as the buttons' tooltip. */
const BROWSER_HISTORY_UNAVAILABLE =
  'Page history needs the desktop app — an embedded frame does not expose its own back/forward to the page around it';

function normalizeUrl(input: string): string {
  const trimmed = input.trim();
  if (!trimmed) return '';
  if (/^(https?|about|file):/i.test(trimmed)) return trimmed;
  if (/^localhost(:\d+)?([/?#]|$)/i.test(trimmed)) return 'http://' + trimmed;
  // Bare hosts ("github.com/foo") get a scheme; anything else — spaces or
  // no dot — is treated as a search query, like a real browser's omnibox.
  if (!/\s/.test(trimmed) && /^[^\s/]+\.[^\s]{2,}/.test(trimmed)) return 'https://' + trimmed;
  return 'https://www.google.com/search?q=' + encodeURIComponent(trimmed);
}

const BrowserPane: React.FC<BrowserPaneProps> = ({
  paneId,
  title,
  isActive,
  initialUrl,
  appMode,
  hibernated,
  onUrlChange,
  pluginId,
}) => {
  const { config } = useConfig();
  const browserCfg = config.browser ?? { homepage: 'https://google.com', bookmarks: [] };

  const [url, setUrl] = useState<string>(initialUrl || browserCfg.homepage || 'https://google.com');
  const [loading, setLoading] = useState(false);
  // Dead-guest state: crashed renderer or unreachable server (plugin sidecar
  // stopped/updating). Rendered as an overlay with a retry, cleared on any
  // successful load. See the render-process-gone/did-fail-load handlers.
  const [guestError, setGuestError] = useState<null | {
    kind: 'crashed' | 'unreachable' | 'blocked';
    detail: string;
    /** Set when the refused target was markdown: the path the preview pane wants. */
    previewPath?: string;
  }>(null);
  const [canGoBack, setCanGoBack] = useState(false);
  const [canGoForward, setCanGoForward] = useState(false);
  const [syncing, setSyncing] = useState(false);
  /** Transient toolbar status for the cookie sync (replaces the old alert()). */
  const [syncMsg, setSyncMsg] = useState<{ text: string; isError: boolean } | null>(null);
  const syncMsgTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [urlFocused, setUrlFocused] = useState(false);
  // ── Guest host ──
  //
  // `<webview>` on the desktop, `<iframe>` on /app. Constant for the life of the
  // build (it reads the installed backend's platform), so it can be resolved
  // once here and read from effects with empty deps.
  const isIframe = guestHost() === 'iframe';
  /** Where the iframe has been navigated since mount; '' = still on startUrl. */
  const [frameSrc, setFrameSrc] = useState('');
  /** Bumped to remount the iframe, which is the only way to force it to reload. */
  const [reloadNonce, setReloadNonce] = useState(0);
  /** The generic-browsing framing caveat is dismissible — on a site that frames
   *  fine it has already said everything it has to say. The plugin-pane notice
   *  is not: that loss is permanent for as long as the pane is open. */
  const [noticeDismissed, setNoticeDismissed] = useState(false);
  const webviewRef = useRef<HTMLElement | null>(null);
  const readyRef = useRef(false);
  const onUrlChangeRef = useRef(onUrlChange);
  onUrlChangeRef.current = onUrlChange;

  // ── Theme bridge (plugin/appMode webviews only) ──
  //
  // Plugin pages are separate documents, so the app's --wks-* vars don't
  // reach them. Inject the full token set (plus color-scheme and
  // zero-specificity body defaults) on every page load and theme change.
  // Regular browsing (appMode=false) is never touched.
  //
  // NOTE: hooks/usePluginWebview.ts is the same contract, extracted for the
  // widget board's guests. This copy stays inline because readyRef below is
  // shared with the key forwarder, whose injection order matters — see that
  // file's KNOWN DUPLICATION note before changing either.
  const { theme } = useTheme();
  const themeRef = useRef(theme);
  themeRef.current = theme;
  const insertedCssKeyRef = useRef<string | null>(null);

  const applyWebviewTheme = useCallback(
    async (newDocument = false) => {
      if (!appMode) return;
      const wv = webviewRef.current as any;
      if (!wv || !readyRef.current) return;
      try {
        // insertCSS keys die with the document they were inserted into
        if (newDocument) insertedCssKeyRef.current = null;
        if (insertedCssKeyRef.current && wv.removeInsertedCSS) {
          await wv.removeInsertedCSS(insertedCssKeyRef.current).catch(() => {});
          insertedCssKeyRef.current = null;
        }
        insertedCssKeyRef.current = await wv.insertCSS(webviewThemeCSS(themeRef.current));
        await wv.executeJavaScript(webviewThemeJS(themeRef.current));
      } catch {
        /* webview mid-navigation or destroyed — next dom-ready re-applies */
      }
    },
    [appMode],
  );
  const applyWebviewThemeRef = useRef(applyWebviewTheme);
  applyWebviewThemeRef.current = applyWebviewTheme;

  // Re-inject when the user switches theme while the plugin is open
  useEffect(() => {
    applyWebviewTheme();
  }, [theme, applyWebviewTheme]);

  // ── Settings bridge (plugin/appMode webviews only) ──
  // Inject the plugin's saved settings as window.__WKS_SETTINGS__ + a
  // wks-settings event, on load and whenever they change, so the plugin applies
  // them live (e.g. the editor toggling vim mode) without a reload.
  const applyWebviewSettings = useCallback(async () => {
    if (!appMode || !pluginId) return;
    const wv = webviewRef.current as any;
    if (!wv || !readyRef.current) return;
    try {
      const values = (await window.electronAPI.getPluginSettings?.(pluginId)) ?? {};
      await wv.executeJavaScript(webviewSettingsJS(values));
    } catch {
      /* webview mid-navigation — re-applied on next dom-ready */
    }
  }, [appMode, pluginId]);
  const applyWebviewSettingsRef = useRef(applyWebviewSettings);
  applyWebviewSettingsRef.current = applyWebviewSettings;

  useEffect(() => {
    if (!pluginId) return;
    const off = window.electronAPI.onPluginSettingsChanged?.((changedId) => {
      if (changedId === pluginId) applyWebviewSettingsRef.current();
    });
    return () => off?.();
  }, [pluginId]);

  // App chords a focused guest must never swallow, DERIVED from the live
  // keybinding config (defaults + user overrides) so presets and rebinds are
  // always covered. The old hardcoded list silently drifted: mod+p (open-file)
  // wasn't in it, so with focus in a webview Ctrl+P fell through to Chromium's
  // print flow — "my keyboard is dead". Only bare mod+<key> combos need
  // enumerating: mod+shift+<anything> and alt-arrows are forwarded wholesale
  // in the matcher, leader chords start with a bare prefix press the guest may
  // keep, and presets never bind guest-essential editing chords (mod+c/v/x/…).
  const { appModKeys, appFKeys } = useMemo(() => {
    const merged = {
      ...DEFAULT_CONFIG.keybindings?.shortcuts,
      ...config.keybindings?.shortcuts,
    } as Record<string, string>;
    const mod = new Set<string>();
    const fkeys = new Set<string>();
    for (const combo of Object.values(merged)) {
      if (!combo || combo.includes(' ')) continue; // leader chords
      const parts = String(combo).toLowerCase().split('+');
      const key = parts[parts.length - 1];
      if (!key) continue;
      if (parts.length === 1 && /^f\d{1,2}$/.test(key)) {
        fkeys.add(key);
        continue;
      }
      const hasMod = ['mod', 'ctrl', 'cmd', 'meta'].some((m) => parts.includes(m));
      if (!hasMod || parts.includes('shift') || parts.includes('alt')) continue;
      mod.add(key);
    }
    fkeys.add('f2'); // pane rename — bound per-pane, not in the shortcuts map
    return { appModKeys: mod, appFKeys: fkeys };
  }, [config.keybindings?.shortcuts]);
  const appModKeysRef = useRef(appModKeys);
  appModKeysRef.current = appModKeys;
  const appFKeysRef = useRef(appFKeys);
  appFKeysRef.current = appFKeys;

  // The resolved chord leader, pre-parsed for the before-input matcher. Null
  // for a lone-modifier leader (Linux Alt tap — armed on the host's key-up,
  // no forwarding needed) and for modifier-less leaders (guest keeps those:
  // typing into a page must never arm a chord).
  const leaderParts = useMemo(() => {
    const resolved = resolveMod(
      resolveLeader(config.keybindings?.prefix ?? 'ctrl+space'),
    ).toLowerCase();
    const parts = resolved.split('+');
    const key = parts[parts.length - 1];
    const mods = {
      ctrl: parts.includes('ctrl'),
      alt: parts.includes('alt'),
      shift: parts.includes('shift'),
      meta: parts.includes('meta'),
    };
    if (!mods.ctrl && !mods.alt && !mods.meta) return null; // bare or lone-mod leader
    if (['ctrl', 'alt', 'shift', 'meta'].includes(key)) return null;
    return { key: key === 'space' ? ' ' : key, ...mods };
  }, [config.keybindings?.prefix]);
  const leaderPartsRef = useRef(leaderParts);
  leaderPartsRef.current = leaderParts;

  // Keep the injected fallback's key sets current across rebinds (the injected
  // listener itself registers once per document; only its data needs refresh).
  useEffect(() => {
    const wv = webviewRef.current as any;
    if (!wv || !readyRef.current) return;
    try {
      wv.executeJavaScript(
        `window.__wksModKeys = ${JSON.stringify([...appModKeys])}; ` +
          `window.__wksFKeys = ${JSON.stringify([...appFKeys])};`,
      );
    } catch {
      /* webview mid-navigation — dom-ready re-injects with current sets */
    }
  }, [appModKeys, appFKeys]);

  // Attach webview event listeners once the element is ready
  useEffect(() => {
    const wv = webviewRef.current as any;
    if (!wv) return;

    const handleDomReady = () => {
      readyRef.current = true;
      // Fresh document: theme first so the page doesn't flash unstyled
      applyWebviewThemeRef.current(true);
      // Then the plugin's settings, so it configures itself on first paint.
      applyWebviewSettingsRef.current();
      // Inject key forwarder once DOM is ready
      setTimeout(() => injectKeyForwarder(), 100);
    };

    // Intercept keyboard shortcuts before the webview page handles them.
    // Electron <webview> fires before-input-event with (event) where
    // event has .key, .type, .control, .alt, .shift, .meta properties.
    const handleBeforeInput = (e: any) => {
      const inp = e;
      if (!inp || inp.type !== 'keyDown') return;

      const key = String(inp.key ?? '').toLowerCase();
      // The chord leader itself must escape the guest, or a chord can never be
      // ARMED while a webview has focus (a combo leader like Ctrl+Space is a
      // bare mod+key the sets below don't cover). Matched against the resolved
      // leader's parts; the Linux lone-Alt tap needs no forwarding here — it
      // fires on the HOST window's key-up, which webview focus doesn't eat.
      const leader = leaderPartsRef.current;
      const isLeader =
        leader !== null &&
        key === leader.key &&
        (inp.control ?? false) === leader.ctrl &&
        (inp.alt ?? false) === leader.alt &&
        (inp.shift ?? false) === leader.shift &&
        (inp.meta ?? false) === leader.meta;
      const isAppShortcut =
        // While a chord is armed, EVERY key belongs to the host — the next
        // keystroke is a chord step (bare letters included), not page input.
        isLayerArmed() ||
        isLeader ||
        ((inp.control || inp.meta) && !inp.alt && !inp.shift && appModKeysRef.current.has(key)) ||
        (inp.control && inp.alt && (inp.key === 'ArrowLeft' || inp.key === 'ArrowRight')) ||
        ((inp.control || inp.meta) && inp.shift) ||
        (inp.alt &&
          !inp.control &&
          ['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(inp.key)) ||
        appFKeysRef.current.has(key);

      if (isAppShortcut) {
        if (e?.preventDefault) e.preventDefault();
        wv.blur();
        window.dispatchEvent(
          new KeyboardEvent('keydown', {
            key: inp.key,
            code: /^[1-9]$/.test(inp.key) ? `Digit${inp.key}` : inp.key,
            ctrlKey: inp.control ?? false,
            altKey: inp.alt ?? false,
            shiftKey: inp.shift ?? false,
            metaKey: inp.meta ?? false,
            bubbles: true,
            cancelable: true,
          }),
        );
      }
    };

    wv.addEventListener('before-input-event', handleBeforeInput);

    // Fallback: inject a key forwarder into the webview content.
    // Some Electron versions don't fire before-input-event reliably on <webview>.
    const injectKeyForwarder = () => {
      try {
        wv.executeJavaScript(`
          window.__wksModKeys = ${JSON.stringify([...appModKeysRef.current])};
          window.__wksFKeys = ${JSON.stringify([...appFKeysRef.current])};
          if (!window.__wksKeyForwarder) {
            window.__wksKeyForwarder = true;
            document.addEventListener('keydown', (e) => {
              const key = String(e.key || '').toLowerCase();
              const isApp = (
                ((e.ctrlKey || e.metaKey) && !e.altKey && !e.shiftKey && (window.__wksModKeys || []).includes(key)) ||
                (e.ctrlKey && e.altKey && (e.key === 'ArrowLeft' || e.key === 'ArrowRight')) ||
                ((e.ctrlKey || e.metaKey) && e.shiftKey) ||
                (e.altKey && !e.ctrlKey && ['ArrowLeft','ArrowRight','ArrowUp','ArrowDown'].includes(e.key)) ||
                (window.__wksFKeys || []).includes(key)
              );
              if (isApp) {
                e.preventDefault();
                e.stopPropagation();
                // Send via console with a special prefix so the host can intercept
                console.log('__WKS_KEY__' + JSON.stringify({
                  key: e.key, ctrl: e.ctrlKey, alt: e.altKey, shift: e.shiftKey, meta: e.metaKey
                }));
              }
            }, true);
          }
        `);
      } catch {}
    };

    // Listen for forwarded keys via console-message
    const handleConsoleMessage = (e: any) => {
      const msg = e?.message ?? '';
      if (!msg.startsWith('__WKS_KEY__')) return;
      try {
        const data = JSON.parse(msg.slice('__WKS_KEY__'.length));
        wv.blur();
        window.dispatchEvent(
          new KeyboardEvent('keydown', {
            key: data.key,
            code: /^[1-9]$/.test(data.key) ? `Digit${data.key}` : data.key,
            ctrlKey: data.ctrl ?? false,
            altKey: data.alt ?? false,
            shiftKey: data.shift ?? false,
            metaKey: data.meta ?? false,
            bubbles: true,
            cancelable: true,
          }),
        );
      } catch {}
    };

    wv.addEventListener('console-message', handleConsoleMessage);

    const handleStartLoading = () => setLoading(true);
    const handleStopLoading = () => {
      setLoading(false);
      setCanGoBack(wv.canGoBack());
      setCanGoForward(wv.canGoForward());
    };

    const handleNavigate = (e: any) => {
      setUrl(e.url);
      setCanGoBack(wv.canGoBack());
      setCanGoForward(wv.canGoForward());
      onUrlChangeRef.current?.(e.url);
    };

    const handleNavigateInPage = (e: any) => {
      setUrl(e.url);
      setCanGoBack(wv.canGoBack());
      setCanGoForward(wv.canGoForward());
      onUrlChangeRef.current?.(e.url);
    };

    // A dead guest must fail VISIBLY and hand focus back to the app. A crashed
    // or unreachable webview otherwise keeps its dead <webview> mounted and
    // focused, silently swallowing every keystroke — for plugin panes (kill or
    // update the sidecar and the guest dies) this read as "workspacer stopped
    // responding / I can't type".
    const reclaimFocus = () => {
      try {
        if (document.activeElement === wv) (wv as unknown as HTMLElement).blur();
      } catch {
        /* focus already elsewhere */
      }
    };
    const handleGone = (e: any) => {
      setLoading(false);
      setGuestError({ kind: 'crashed', detail: e?.reason || 'render process gone' });
      reclaimFocus();
    };
    const handleFailLoad = (e: any) => {
      // Subframes and benign aborts (-3: navigation superseded) don't count.
      if (e && e.isMainFrame === false) return;
      if (e && e.errorCode === -3) return;
      setLoading(false);
      setGuestError({
        kind: 'unreachable',
        detail: (e?.errorDescription || 'load failed') + (e?.errorCode ? ` (${e.errorCode})` : ''),
      });
      reclaimFocus();
    };
    const handleFinishLoad = () => setGuestError(null);
    wv.addEventListener('render-process-gone', handleGone);
    wv.addEventListener('crashed', handleGone); // older <webview> event name
    wv.addEventListener('did-fail-load', handleFailLoad);
    wv.addEventListener('did-finish-load', handleFinishLoad);

    // An <iframe> fires none of the events above, but it does fire a plain
    // `load` — enough to clear the progress bar. It is deliberately NOT wired to
    // `guestError`: a frame refused by X-Frame-Options also fires `load`, so
    // treating load as success would be a lie, and there is no event that
    // reports the refusal (see the embedded-view notice in the toolbar).
    const handleFrameLoad = () => setLoading(false);
    if (isIframe) wv.addEventListener('load', handleFrameLoad);

    wv.addEventListener('dom-ready', handleDomReady);
    wv.addEventListener('did-start-loading', handleStartLoading);
    wv.addEventListener('did-stop-loading', handleStopLoading);
    wv.addEventListener('did-navigate', handleNavigate);
    wv.addEventListener('did-navigate-in-page', handleNavigateInPage);

    return () => {
      if (isIframe) wv.removeEventListener('load', handleFrameLoad);
      wv.removeEventListener('dom-ready', handleDomReady);
      wv.removeEventListener('before-input-event', handleBeforeInput);
      wv.removeEventListener('console-message', handleConsoleMessage);
      wv.removeEventListener('did-start-loading', handleStartLoading);
      wv.removeEventListener('did-stop-loading', handleStopLoading);
      wv.removeEventListener('did-navigate', handleNavigate);
      wv.removeEventListener('did-navigate-in-page', handleNavigateInPage);
      wv.removeEventListener('render-process-gone', handleGone);
      wv.removeEventListener('crashed', handleGone);
      wv.removeEventListener('did-fail-load', handleFailLoad);
      wv.removeEventListener('did-finish-load', handleFinishLoad);
    };
    // reloadNonce is the iframe refresh: it REMOUNTS the element (the only way
    // to reload a cross-origin frame), so the listeners have to re-attach to the
    // new one or the progress bar never clears again. It never changes on the
    // desktop, where refresh is wv.reload() and the element survives.
  }, [reloadNonce]);

  // Compute the start URL once for the webview src attribute
  const startUrl = normalizeUrl(initialUrl || browserCfg.homepage || 'https://google.com');
  /** What the guest is actually showing — the source of the framing policy. */
  const guestSrc = frameSrc || startUrl;
  const framePolicy = useMemo(
    () => guestFramePolicy(guestSrc, isIframe ? 'iframe' : 'webview'),
    [guestSrc, isIframe],
  );

  const navigate = useCallback(
    (targetUrl: string) => {
      const normalized = normalizeUrl(targetUrl);
      if (!normalized) return;
      setUrl(normalized);
      // An iframe has no loadURL; it navigates by having its src replaced. The
      // guest also can't tell us where it went afterwards (cross-origin), so the
      // omnibox is authoritative rather than reflective here.
      if (isIframe) {
        setFrameSrc(normalized);
        setLoading(true);
        onUrlChangeRef.current?.(normalized);
        return;
      }
      const wv = webviewRef.current as any;
      if (wv && wv.loadURL) {
        wv.loadURL(normalized);
      }
    },
    [isIframe],
  );

  // ── Refusals from the main-process webview guard ──
  //
  // A prevented ATTACH destroys the guest before it exists, so no did-fail-load
  // ever fires and the pane is simply an empty rectangle. This push is the only
  // signal that the blankness is a decision rather than a bug, and it is what
  // turns it into the banner below.
  const ownUrlsRef = useRef<{ start: string; current: string }>({ start: '', current: '' });
  ownUrlsRef.current = { start: startUrl, current: normalizeUrl(url) };
  /** This pane's guest, when it has one. `getWebContentsId` throws until the
   *  <webview> has attached, and an <iframe> has none at all. */
  const guestWebContentsId = useCallback((): number | null => {
    const wv = webviewRef.current as any;
    try {
      return typeof wv?.getWebContentsId === 'function' ? wv.getWebContentsId() : null;
    } catch {
      return null;
    }
  }, []);
  useEffect(() => {
    const off = window.electronAPI.onWebviewBlocked?.((info) => {
      const mine = ownUrlsRef.current;
      // Every pane hears every refusal; only the one it is ABOUT should claim it.
      //
      // Identity first. Matching on the URL alone silently failed for the case
      // the banner most needs to explain: Chromium reports the WHATWG-normalised
      // href, so a typed `file:///home/x/../y.html`, a `%2e%2e` segment, an
      // uppercase `FILE:///`, `http://EXAMPLE.com` and an unencoded space all
      // came back different from the string the pane was holding, the refusal
      // was claimed by nobody, and the address-bar traversal stayed a silent
      // blank.
      //
      // The URL comparison is the fallback, not the rule: a refused ATTACH
      // carries no guest id (there is no guest yet), and a guest that has not
      // attached cannot report its own. It compares normalised hrefs on both
      // sides, so those five spellings match there too.
      const myId = guestWebContentsId();
      if (info.webContentsId != null && myId != null) {
        if (info.webContentsId !== myId) return;
      } else if (!sameUrl(info.url, mine.start) && !sameUrl(info.url, mine.current)) {
        return;
      }
      setLoading(false);
      setGuestError({
        kind: 'blocked',
        detail: `Blocked: ${info.reason}`,
        // MAIN decides whether a refused target may open in the preview pane:
        // it is the only side that knows the roots. Deriving it here from the
        // URL's extension offered an out-of-root .md a button that walked
        // straight around the guard that had just refused it.
        previewPath: info.previewPath,
      });
    });
    return () => off?.();
  }, [guestWebContentsId]);

  const handleGo = useCallback(() => {
    navigate(url);
  }, [url, navigate]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === 'Enter') {
        e.preventDefault();
        handleGo();
      }
    },
    [handleGo],
  );

  const handleRefresh = useCallback(() => {
    // Remounting is the only reliable reload for a cross-origin iframe: setting
    // src to the value it already has is a no-op, and reload() lives on the
    // guest's contentWindow, which same-origin policy keeps out of reach.
    if (isIframe) {
      setLoading(true);
      setReloadNonce((n) => n + 1);
      return;
    }
    const wv = webviewRef.current as any;
    if (wv && wv.reload) wv.reload();
  }, [isIframe]);

  const handleStopLoad = useCallback(() => {
    const wv = webviewRef.current as any;
    if (wv && wv.stop) wv.stop();
  }, []);

  const handleBack = useCallback(() => {
    const wv = webviewRef.current as any;
    if (wv && wv.canGoBack && wv.canGoBack()) wv.goBack();
  }, []);

  const handleForward = useCallback(() => {
    const wv = webviewRef.current as any;
    if (wv && wv.canGoForward && wv.canGoForward()) wv.goForward();
  }, []);

  const handleOpenExternal = useCallback(() => {
    const normalized = normalizeUrl(url);
    if (normalized) window.open(normalized, '_blank');
  }, [url]);

  const showSyncMsg = useCallback((text: string, isError: boolean) => {
    if (syncMsgTimerRef.current) clearTimeout(syncMsgTimerRef.current);
    setSyncMsg({ text, isError });
    syncMsgTimerRef.current = setTimeout(() => setSyncMsg(null), 6000);
  }, []);
  useEffect(
    () => () => {
      if (syncMsgTimerRef.current) clearTimeout(syncMsgTimerRef.current);
    },
    [],
  );

  const handleSyncChromeCookies = useCallback(async () => {
    setSyncing(true);
    try {
      // Restrict the import to hosts likely to be relevant for sign-in flows.
      // The wildcard list keeps Chrome's bag of unrelated cookies (banking,
      // shopping, etc.) out of Workspacer's session.
      const res = await window.electronAPI.importChromeCookies([
        'atlassian.com',
        'atlassian.net',
        'microsoftonline.com',
        'microsoft.com',
        'live.com',
        'office.com',
        'office365.com',
        'google.com',
        'github.com',
      ]);
      // Reload the current page so any session that depended on the new cookies takes effect.
      const wv = webviewRef.current as any;
      if (wv && typeof wv.reload === 'function') wv.reload();
      const summary = `Imported ${res.imported} cookie(s), skipped ${res.skipped}`;
      if (res.errors.length === 0) {
        showSyncMsg(summary, false);
      } else {
        showSyncMsg(`${summary} — ${res.errors[0]}`, true);
      }
    } catch (err: any) {
      showSyncMsg(`Cookie sync failed: ${err?.message ?? err}`, true);
    } finally {
      setSyncing(false);
    }
  }, [showSyncMsg]);

  const bookmarks: Bookmark[] = browserCfg.bookmarks ?? [];
  const isSecure = /^https:/i.test(url);

  return (
    <div
      style={{
        position: 'relative',
        display: 'flex',
        flexDirection: 'column',
        height: '100%',
        backgroundColor: 'var(--wks-bg-base)',
        color: 'var(--wks-text-primary)',
        fontFamily: 'var(--wks-font-sans)',
        fontSize: '0.75rem',
      }}
    >
      {/* URL bar + bookmarks — hidden in app mode */}
      {!appMode && (
        <>
          <div
            style={{
              position: 'relative',
              display: 'flex',
              alignItems: 'center',
              gap: '2px',
              padding: '5px 8px',
              backgroundColor: 'var(--wks-bg-surface)',
              borderBottom: '1px solid var(--wks-border-subtle)',
            }}
          >
            {/* Back/forward/stop drive the guest's own session history, which an
                iframe does not expose to its embedder. Disabled with a reason —
                a live button that silently does nothing is the failure mode this
                fallback exists to remove. */}
            <NavButton
              onClick={handleBack}
              title={isIframe ? BROWSER_HISTORY_UNAVAILABLE : 'Back'}
              disabled={isIframe || !canGoBack}
            >
              <IconBack />
            </NavButton>
            <NavButton
              onClick={handleForward}
              title={isIframe ? BROWSER_HISTORY_UNAVAILABLE : 'Forward'}
              disabled={isIframe || !canGoForward}
            >
              <IconForward />
            </NavButton>
            <NavButton
              onClick={loading && !isIframe ? handleStopLoad : handleRefresh}
              title={loading && !isIframe ? 'Stop loading' : 'Refresh'}
            >
              {loading && !isIframe ? <IconStop /> : <IconRefresh />}
            </NavButton>

            {/* Omnibox pill */}
            <div
              style={{
                flex: 1,
                display: 'flex',
                alignItems: 'center',
                gap: '7px',
                height: '26px',
                margin: '0 4px',
                padding: '0 11px',
                minWidth: 0,
                backgroundColor: 'var(--wks-bg-input)',
                border: `1px solid ${urlFocused ? 'var(--wks-border-active)' : 'var(--wks-border-input)'}`,
                borderRadius: 'var(--wks-radius-pill)',
                boxShadow: urlFocused ? '0 0 0 2px var(--wks-accent-glow)' : 'none',
                transition: 'border-color 0.12s ease, box-shadow 0.12s ease',
              }}
            >
              <span
                title={isSecure ? 'Secure (https)' : 'Not secure'}
                style={{
                  display: 'flex',
                  flexShrink: 0,
                  color: isSecure ? 'var(--wks-text-faint)' : 'var(--wks-warning)',
                }}
              >
                <IconLock open={!isSecure} />
              </span>
              <input
                type="text"
                value={url}
                onChange={(e) => setUrl(e.target.value)}
                onKeyDown={handleKeyDown}
                onFocus={(e) => {
                  setUrlFocused(true);
                  e.currentTarget.select();
                }}
                onBlur={() => setUrlFocused(false)}
                placeholder="Search or enter URL…"
                spellCheck={false}
                style={{
                  flex: 1,
                  minWidth: 0,
                  background: 'transparent',
                  border: 'none',
                  outline: 'none',
                  fontSize: '0.7rem',
                  fontFamily: 'var(--wks-font-mono)',
                  color: 'var(--wks-text-primary)',
                }}
              />
            </div>

            {syncMsg && (
              <span
                title={syncMsg.text}
                style={{
                  maxWidth: '180px',
                  overflow: 'hidden',
                  textOverflow: 'ellipsis',
                  whiteSpace: 'nowrap',
                  fontSize: '0.66rem',
                  fontWeight: 600,
                  color: syncMsg.isError ? 'var(--wks-error)' : 'var(--wks-text-muted)',
                  animation: 'wks-fade-in 0.2s ease',
                }}
              >
                {syncMsg.text}
              </span>
            )}

            <ToolbarSep />
            <NavButton onClick={handleOpenExternal} title="Open in system browser">
              <IconExternal />
            </NavButton>
            <NavButton
              onClick={handleSyncChromeCookies}
              title={
                isIframe
                  ? 'Cookie sync needs the desktop app — it writes into Electron’s browser session, which a browser tab has no equivalent of'
                  : 'Sync sign-in cookies from Chrome (fixes stubborn OAuth flows)'
              }
              disabled={isIframe || syncing}
              spinning={syncing}
            >
              <IconCookie />
            </NavButton>

            {loading && <div className="wks-browser-progress" />}
          </div>

          {bookmarks.length > 0 && (
            <div
              style={{
                display: 'flex',
                flexWrap: 'wrap',
                gap: '2px',
                padding: '3px 8px',
                backgroundColor: 'var(--wks-bg-surface)',
                borderBottom: '1px solid var(--wks-border-subtle)',
              }}
            >
              {bookmarks.map((bm, i) => (
                <button
                  key={i}
                  onClick={() => navigate(bm.url)}
                  title={bm.url}
                  style={{
                    padding: '2px 8px',
                    fontSize: '0.68rem',
                    fontWeight: 600,
                    fontFamily: 'inherit',
                    backgroundColor: 'transparent',
                    color: 'var(--wks-text-secondary)',
                    border: 'none',
                    borderRadius: 'var(--wks-radius-pill)',
                    cursor: 'pointer',
                    whiteSpace: 'nowrap',
                    lineHeight: '14px',
                  }}
                  onMouseEnter={(e) => {
                    e.currentTarget.style.backgroundColor = 'var(--wks-bg-hover)';
                    e.currentTarget.style.color = 'var(--wks-accent-text)';
                  }}
                  onMouseLeave={(e) => {
                    e.currentTarget.style.backgroundColor = 'transparent';
                    e.currentTarget.style.color = 'var(--wks-text-secondary)';
                  }}
                >
                  {bm.name}
                </button>
              ))}
            </div>
          )}
        </>
      )}

      {/* App mode has no toolbar to host the progress bar — pin it to the pane top */}
      {appMode && loading && (
        <div className="wks-browser-progress" style={{ top: 0, bottom: 'auto' }} />
      )}

      {/* Content area */}
      {hibernated ? (
        <div
          style={{
            flex: 1,
            display: 'flex',
            flexDirection: 'column',
            alignItems: 'center',
            justifyContent: 'center',
            backgroundColor: 'var(--wks-bg-base)',
            color: 'var(--wks-text-muted)',
            gap: '8px',
          }}
        >
          <span style={{ fontSize: '2rem', opacity: 0.5 }}>&#x1F4A4;</span>
          <span style={{ fontSize: '0.7rem' }}>Hibernated</span>
          <span
            style={{
              fontSize: '0.64rem',
              fontFamily: 'var(--wks-font-mono)',
              color: 'var(--wks-text-faint)',
            }}
          >
            {url}
          </span>
        </div>
      ) : (
        <div
          style={{
            flex: 1,
            position: 'relative',
            display: 'flex',
            minWidth: 0,
            flexDirection: 'column',
          }}
        >
          {/* What the iframe genuinely cannot do, said out loud. Two different
              losses, so two different sentences — a generic "limited on the web"
              would leave the user guessing which one they just hit. */}
          {isIframe && appMode && !framePolicy.canReachBus && (
            <GuestNotice>
              This plugin&rsquo;s UI is served from the same address as workspacer, so the browser
              runs it sandboxed and it cannot open its hub connection. It will show its static
              interface only — open this pane in the desktop app for the live version.
            </GuestNotice>
          )}
          {isIframe && !appMode && !noticeDismissed && (
            <GuestNotice onDismiss={() => setNoticeDismissed(true)}>
              Embedded view. Sites that refuse to be framed (most large ones) stay blank here — use{' '}
              <button
                onClick={handleOpenExternal}
                style={{
                  background: 'none',
                  border: 'none',
                  padding: 0,
                  font: 'inherit',
                  color: 'var(--wks-accent-text)',
                  cursor: 'pointer',
                  textDecoration: 'underline',
                }}
              >
                open in system browser
              </button>{' '}
              for those.
            </GuestNotice>
          )}
          <GuestFrame
            key={reloadNonce}
            ref={webviewRef}
            src={isIframe ? guestSrc : startUrl}
            title={title}
            style={{
              flex: 1,
              width: '100%',
              border: 'none',
            }}
            partition="persist:browser"
            allowPopups
          />
          {guestError && (
            <div
              style={{
                position: 'absolute',
                inset: 0,
                display: 'flex',
                flexDirection: 'column',
                alignItems: 'center',
                justifyContent: 'center',
                gap: 8,
                background: 'var(--wks-bg-base)',
                color: 'var(--wks-text-muted)',
                textAlign: 'center',
                padding: 24,
              }}
            >
              <span style={{ fontSize: '1.6rem', opacity: 0.6 }}>
                {guestError.kind === 'crashed'
                  ? '\u{1F4A5}'
                  : guestError.kind === 'blocked'
                    ? '\u{1F6AB}'
                    : '\u{1F50C}'}
              </span>
              <span style={{ fontSize: '0.78rem', color: 'var(--wks-text-secondary)' }}>
                {guestError.kind === 'blocked'
                  ? 'Workspacer blocked this URL.'
                  : appMode
                    ? guestError.kind === 'crashed'
                      ? 'This plugin’s UI crashed.'
                      : 'This plugin’s UI can’t be reached — its sidecar may be stopped or restarting.'
                    : guestError.kind === 'crashed'
                      ? 'This page crashed.'
                      : 'This page can’t be reached.'}
              </span>
              <span
                style={{
                  fontSize: '0.62rem',
                  fontFamily: 'var(--wks-font-mono)',
                  color: 'var(--wks-text-faint)',
                }}
              >
                {guestError.detail}
              </span>
              {/* A refusal is a decision, not a transient failure: retrying loads
                  the same URL into the same guard. The only useful button is the
                  one that opens the surface which CAN show the target, offered
                  when the refusal was a markdown file. */}
              {guestError.kind !== 'blocked' && (
                <button
                  onClick={() => {
                    setGuestError(null);
                    const wv = webviewRef.current as any;
                    try {
                      if (wv?.loadURL) wv.loadURL(normalizeUrl(url) || startUrl);
                      else wv?.reload?.();
                    } catch {
                      /* webview mid-teardown — the overlay returns on next failure */
                    }
                  }}
                  style={GUEST_ERROR_BUTTON}
                >
                  Retry
                </button>
              )}
              {guestError.kind === 'blocked' && guestError.previewPath && (
                <button
                  onClick={() => requestMarkdownPreview({ path: guestError.previewPath as string })}
                  style={GUEST_ERROR_BUTTON}
                >
                  Open markdown preview
                </button>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  );
};

/** Shared look for the dead-guest overlay's single action button. */
const GUEST_ERROR_BUTTON: React.CSSProperties = {
  marginTop: 4,
  padding: '5px 14px',
  fontSize: '0.7rem',
  border: '1px solid var(--wks-border-subtle)',
  borderRadius: 'var(--wks-radius-md)',
  background: 'var(--wks-bg-raised)',
  color: 'var(--wks-text-primary)',
  cursor: 'pointer',
};

/** A slim strip above the guest stating what this host cannot do for it. */
const GuestNotice: React.FC<{ children: React.ReactNode; onDismiss?: () => void }> = ({
  children,
  onDismiss,
}) => (
  <div
    style={{
      display: 'flex',
      alignItems: 'flex-start',
      gap: 6,
      flexShrink: 0,
      padding: '6px 10px',
      fontSize: '0.66rem',
      lineHeight: 1.45,
      color: 'var(--wks-text-secondary)',
      backgroundColor: 'color-mix(in srgb, var(--wks-warning) 10%, transparent)',
      borderBottom: '1px solid var(--wks-border-subtle)',
    }}
  >
    <AlertTriangle
      size={12}
      strokeWidth={2}
      style={{ flexShrink: 0, marginTop: 2, color: 'var(--wks-warning)' }}
    />
    <span style={{ flex: 1 }}>{children}</span>
    {onDismiss && (
      <button
        onClick={onDismiss}
        title="Dismiss"
        style={{
          display: 'flex',
          flexShrink: 0,
          padding: 0,
          background: 'none',
          border: 'none',
          color: 'var(--wks-text-muted)',
          cursor: 'pointer',
        }}
      >
        <X size={12} strokeWidth={2} />
      </button>
    )}
  </div>
);

/** Flat toolbar icon button — borderless, hover-raised, like the composer controls. */
const NavButton: React.FC<{
  onClick: () => void;
  title: string;
  disabled?: boolean;
  spinning?: boolean;
  children: React.ReactNode;
}> = ({ onClick, title, disabled, spinning, children }) => (
  <button
    onClick={onClick}
    title={title}
    disabled={disabled}
    style={{
      height: '24px',
      width: '26px',
      padding: 0,
      flexShrink: 0,
      display: 'flex',
      alignItems: 'center',
      justifyContent: 'center',
      backgroundColor: 'transparent',
      color: disabled ? 'var(--wks-text-disabled)' : 'var(--wks-text-secondary)',
      border: 'none',
      borderRadius: 'var(--wks-radius-sm)',
      cursor: disabled ? 'default' : 'pointer',
      transition: 'background-color 0.1s ease, color 0.1s ease',
    }}
    onMouseEnter={(e) => {
      if (disabled) return;
      e.currentTarget.style.backgroundColor = 'var(--wks-bg-hover)';
      e.currentTarget.style.color = 'var(--wks-text-primary)';
    }}
    onMouseLeave={(e) => {
      e.currentTarget.style.backgroundColor = 'transparent';
      e.currentTarget.style.color = disabled
        ? 'var(--wks-text-disabled)'
        : 'var(--wks-text-secondary)';
    }}
  >
    <span
      style={{ display: 'flex', animation: spinning ? 'wks-spin 0.9s linear infinite' : undefined }}
    >
      {children}
    </span>
  </button>
);

/** Thin vertical rule between toolbar groups (matches the composer's separators). */
const ToolbarSep: React.FC = () => (
  <span
    aria-hidden
    style={{
      width: 1,
      height: 14,
      flexShrink: 0,
      margin: '0 4px',
      background: 'var(--wks-border-subtle)',
    }}
  />
);

// ── Toolbar icons — 14px, stroke-based, inherit currentColor ──

const iconProps = {
  width: 14,
  height: 14,
  viewBox: '0 0 16 16',
  fill: 'none',
  stroke: 'currentColor',
  strokeWidth: 1.5,
  strokeLinecap: 'round' as const,
  strokeLinejoin: 'round' as const,
};

const IconBack: React.FC = () => (
  <svg {...iconProps}>
    <path d="M10 3.5 5.5 8l4.5 4.5" />
  </svg>
);

const IconForward: React.FC = () => (
  <svg {...iconProps}>
    <path d="M6 3.5 10.5 8 6 12.5" />
  </svg>
);

const IconRefresh: React.FC = () => (
  <svg {...iconProps}>
    <path d="M14 8a6 6 0 1 1-6-6c1.68 0 3.29.67 4.49 1.83L14 5.33" />
    <polyline points="14 2 14 5.33 10.67 5.33" />
  </svg>
);

const IconStop: React.FC = () => (
  <svg {...iconProps}>
    <path d="m4.5 4.5 7 7M11.5 4.5l-7 7" />
  </svg>
);

const IconExternal: React.FC = () => (
  <svg {...iconProps}>
    <path d="M4.67 11.33 11.33 4.67" />
    <polyline points="5.33 4.67 11.33 4.67 11.33 10.67" />
  </svg>
);

const IconLock: React.FC<{ open?: boolean }> = ({ open }) => (
  <svg {...iconProps} width={11} height={11} strokeWidth={1.6}>
    <rect x="3" y="7" width="10" height="6.5" rx="1.8" />
    {open ? <path d="M5.5 7V5a2.5 2.5 0 0 1 4.9-.7" /> : <path d="M5.5 7V5a2.5 2.5 0 0 1 5 0v2" />}
  </svg>
);

const IconCookie: React.FC = () => (
  <svg {...iconProps}>
    <path d="M13.9 8.6A6 6 0 1 1 7.4 2.1 2.4 2.4 0 0 0 10 4.7a2.4 2.4 0 0 0 2.6 2.6c.5 0 1 .5 1.3 1.3Z" />
    <circle cx="6" cy="7" r="0.4" fill="currentColor" />
    <circle cx="7.6" cy="10.4" r="0.4" fill="currentColor" />
    <circle cx="10.4" cy="9.6" r="0.4" fill="currentColor" />
  </svg>
);

export default BrowserPane;
