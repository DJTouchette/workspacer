import React, { useState, useCallback, useRef, useEffect } from 'react';
import { useConfig } from '../hooks/useConfig';
import { useTheme } from '../hooks/useTheme';
import { webviewThemeCSS, webviewThemeJS } from '../lib/webviewTheme';

interface BrowserPaneProps {
  paneId: string;
  title: string;
  isActive: boolean;
  initialUrl?: string;
  appMode?: boolean;
  hibernated?: boolean;
  onUrlChange?: (url: string) => void;
}

interface Bookmark {
  name: string;
  url: string;
}

function normalizeUrl(input: string): string {
  const trimmed = input.trim();
  if (!trimmed) return '';
  if (/^https?:\/\//i.test(trimmed)) return trimmed;
  return 'https://' + trimmed;
}

const BrowserPane: React.FC<BrowserPaneProps> = ({ paneId, title, isActive, initialUrl, appMode, hibernated, onUrlChange }) => {
  const { config } = useConfig();
  const browserCfg = config.browser ?? { homepage: 'https://google.com', bookmarks: [] };

  const [url, setUrl] = useState<string>(initialUrl || browserCfg.homepage || 'https://google.com');
  const [loading, setLoading] = useState(false);
  const [canGoBack, setCanGoBack] = useState(false);
  const [canGoForward, setCanGoForward] = useState(false);
  const [syncing, setSyncing] = useState(false);
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
  const { theme } = useTheme();
  const themeRef = useRef(theme);
  themeRef.current = theme;
  const insertedCssKeyRef = useRef<string | null>(null);

  const applyWebviewTheme = useCallback(async (newDocument = false) => {
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
  }, [appMode]);
  const applyWebviewThemeRef = useRef(applyWebviewTheme);
  applyWebviewThemeRef.current = applyWebviewTheme;

  // Re-inject when the user switches theme while the plugin is open
  useEffect(() => {
    applyWebviewTheme();
  }, [theme, applyWebviewTheme]);

  // Attach webview event listeners once the element is ready
  useEffect(() => {
    const wv = webviewRef.current as any;
    if (!wv) return;

    const handleDomReady = () => {
      readyRef.current = true;
      // Fresh document: theme first so the page doesn't flash unstyled
      applyWebviewThemeRef.current(true);
      // Inject key forwarder once DOM is ready
      setTimeout(() => injectKeyForwarder(), 100);
    };

    // Intercept keyboard shortcuts before the webview page handles them.
    // Electron <webview> fires before-input-event with (event) where
    // event has .key, .type, .control, .alt, .shift, .meta properties.
    const handleBeforeInput = (e: any) => {
      const inp = e;
      if (!inp || inp.type !== 'keyDown') return;

      const isAppShortcut = (
        (inp.control && !inp.alt && /^[1-9tbwdsk,/?]$/i.test(inp.key)) ||
        (inp.control && inp.alt && (inp.key === 'ArrowLeft' || inp.key === 'ArrowRight')) ||
        (inp.control && inp.shift) ||
        (inp.alt && !inp.control && ['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(inp.key)) ||
        inp.key === 'F2'
      );

      if (isAppShortcut) {
        if (e?.preventDefault) e.preventDefault();
        wv.blur();
        window.dispatchEvent(new KeyboardEvent('keydown', {
          key: inp.key,
          code: /^[1-9]$/.test(inp.key) ? `Digit${inp.key}` : inp.key,
          ctrlKey: inp.control ?? false,
          altKey: inp.alt ?? false,
          shiftKey: inp.shift ?? false,
          metaKey: inp.meta ?? false,
          bubbles: true,
          cancelable: true,
        }));
      }
    };

    wv.addEventListener('before-input-event', handleBeforeInput);

    // Fallback: inject a key forwarder into the webview content.
    // Some Electron versions don't fire before-input-event reliably on <webview>.
    const injectKeyForwarder = () => {
      try {
        wv.executeJavaScript(`
          if (!window.__wksKeyForwarder) {
            window.__wksKeyForwarder = true;
            document.addEventListener('keydown', (e) => {
              const isApp = (
                (e.ctrlKey && !e.altKey && /^[1-9tbwdsk,/?]$/i.test(e.key)) ||
                (e.ctrlKey && e.altKey && (e.key === 'ArrowLeft' || e.key === 'ArrowRight')) ||
                (e.ctrlKey && e.shiftKey) ||
                (e.altKey && !e.ctrlKey && ['ArrowLeft','ArrowRight','ArrowUp','ArrowDown'].includes(e.key)) ||
                e.key === 'F2'
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
        window.dispatchEvent(new KeyboardEvent('keydown', {
          key: data.key,
          code: /^[1-9]$/.test(data.key) ? `Digit${data.key}` : data.key,
          ctrlKey: data.ctrl ?? false,
          altKey: data.alt ?? false,
          shiftKey: data.shift ?? false,
          metaKey: data.meta ?? false,
          bubbles: true,
          cancelable: true,
        }));
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

    wv.addEventListener('dom-ready', handleDomReady);
    wv.addEventListener('did-start-loading', handleStartLoading);
    wv.addEventListener('did-stop-loading', handleStopLoading);
    wv.addEventListener('did-navigate', handleNavigate);
    wv.addEventListener('did-navigate-in-page', handleNavigateInPage);

    return () => {
      wv.removeEventListener('dom-ready', handleDomReady);
      wv.removeEventListener('before-input-event', handleBeforeInput);
      wv.removeEventListener('console-message', handleConsoleMessage);
      wv.removeEventListener('did-start-loading', handleStartLoading);
      wv.removeEventListener('did-stop-loading', handleStopLoading);
      wv.removeEventListener('did-navigate', handleNavigate);
      wv.removeEventListener('did-navigate-in-page', handleNavigateInPage);
    };
  }, []);

  // Compute the start URL once for the webview src attribute
  const startUrl = normalizeUrl(initialUrl || browserCfg.homepage || 'https://google.com');

  const navigate = useCallback((targetUrl: string) => {
    const normalized = normalizeUrl(targetUrl);
    if (!normalized) return;
    setUrl(normalized);
    const wv = webviewRef.current as any;
    if (wv && wv.loadURL) {
      wv.loadURL(normalized);
    }
  }, []);

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
    const wv = webviewRef.current as any;
    if (wv && wv.reload) wv.reload();
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
      const summary = `Imported ${res.imported} cookie(s), skipped ${res.skipped}`;
      // Reload the current page so any session that depended on the new cookies takes effect.
      const wv = webviewRef.current as any;
      if (wv && typeof wv.reload === 'function') wv.reload();
      // Surface a small visible signal — fall back to alert() if no toast system.
      if (res.errors.length === 0) {
        alert(summary);
      } else {
        alert(`${summary}\n\nFirst error: ${res.errors[0]}`);
      }
    } catch (err: any) {
      alert(`Chrome cookie sync failed: ${err?.message ?? err}`);
    } finally {
      setSyncing(false);
    }
  }, []);

  const bookmarks: Bookmark[] = browserCfg.bookmarks ?? [];

  return (
    <div
      style={{
        display: 'flex',
        flexDirection: 'column',
        height: '100%',
        backgroundColor: '#121214',
        color: '#e4e4e7',
        fontFamily: 'Inter, system-ui, sans-serif',
        fontSize: '12px',
      }}
    >
      {/* URL bar + bookmarks — hidden in app mode */}
      {!appMode && (<>
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: '3px',
            padding: '3px 6px',
            backgroundColor: '#1a1a1e',
            borderBottom: '1px solid #2a2a30',
          }}
        >
          <button
            onClick={handleBack}
            title="Back"
            style={{
              ...navBtnStyle,
              opacity: canGoBack ? 1 : 0.4,
            }}
          >
            &#x2190;
          </button>
          <button
            onClick={handleForward}
            title="Forward"
            style={{
              ...navBtnStyle,
              opacity: canGoForward ? 1 : 0.4,
            }}
          >
            &#x2192;
          </button>
          <button onClick={handleRefresh} title="Refresh" style={navBtnStyle}>&#x21BB;</button>
          <input
            type="text"
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder="Enter URL..."
            spellCheck={false}
            style={{
              flex: 1,
              height: '24px',
              padding: '0 8px',
              fontSize: '11px',
              fontFamily: 'JetBrainsMono NF, JetBrainsMono Nerd Font, monospace',
              backgroundColor: '#0e0e10',
              color: '#e4e4e7',
              border: '1px solid #2a2a30',
              borderRadius: '3px',
              outline: 'none',
            }}
            onFocus={(e) => { e.currentTarget.style.borderColor = '#60a5fa'; }}
            onBlur={(e) => { e.currentTarget.style.borderColor = '#2a2a30'; }}
          />
          <button onClick={handleGo} title="Navigate" style={{ ...navBtnStyle, backgroundColor: '#2563eb', color: '#e4e4e7', fontWeight: 600, padding: '0 8px', width: 'auto' }}>Go</button>
          <button onClick={handleOpenExternal} title="Open in system browser" style={navBtnStyle}>&#x2197;</button>
          <button
            onClick={handleSyncChromeCookies}
            title="Sync cookies from Chrome (fixes OAuth sign-ins)"
            style={navBtnStyle}
            disabled={syncing}
          >
            {syncing ? '⋯' : '\u{1F36A}'}
          </button>
        </div>

        {bookmarks.length > 0 && (
          <div
            style={{
              display: 'flex',
              flexWrap: 'wrap',
              gap: '2px',
              padding: '2px 6px',
              backgroundColor: '#16161a',
              borderBottom: '1px solid #2a2a30',
            }}
          >
            {bookmarks.map((bm, i) => (
              <button
                key={i}
                onClick={() => navigate(bm.url)}
                title={bm.url}
                style={{
                  height: '20px',
                  padding: '0 6px',
                  fontSize: '10px',
                  backgroundColor: '#1e1e22',
                  color: '#a0b4e6',
                  border: '1px solid #2a2a30',
                  borderRadius: '2px',
                  cursor: 'pointer',
                  whiteSpace: 'nowrap',
                  lineHeight: '1',
                }}
                onMouseEnter={(e) => { e.currentTarget.style.backgroundColor = '#2a2a30'; }}
                onMouseLeave={(e) => { e.currentTarget.style.backgroundColor = '#1e1e22'; }}
              >
                {bm.name}
              </button>
            ))}
          </div>
        )}
      </>)}

      {/* Loading indicator */}
      {loading && (
        <div style={{
          height: '2px',
          backgroundColor: '#2563eb',
        }} />
      )}

      {/* Content area */}
      {hibernated ? (
        <div style={{
          flex: 1,
          display: 'flex',
          flexDirection: 'column',
          alignItems: 'center',
          justifyContent: 'center',
          backgroundColor: '#121214',
          color: 'rgb(100, 100, 115)',
          gap: '8px',
        }}>
          <span style={{ fontSize: '2rem', opacity: 0.5 }}>&#x1F4A4;</span>
          <span style={{ fontSize: '0.7rem' }}>Hibernated</span>
          <span style={{ fontSize: '0.6rem', color: 'rgb(70, 70, 80)' }}>{url}</span>
        </div>
      ) : (
        <webview
          ref={webviewRef as any}
          src={startUrl}
          style={{
            flex: 1,
            width: '100%',
            border: 'none',
          }}
          // @ts-ignore
          partition="persist:browser"
          // @ts-ignore
          allowpopups="true"
        />
      )}
    </div>
  );
};

const navBtnStyle: React.CSSProperties = {
  height: '24px',
  width: '24px',
  padding: 0,
  fontSize: '12px',
  backgroundColor: '#27272a',
  color: '#a1a1aa',
  border: '1px solid #2a2a30',
  borderRadius: '3px',
  cursor: 'pointer',
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'center',
  lineHeight: '1',
};

export default BrowserPane;
