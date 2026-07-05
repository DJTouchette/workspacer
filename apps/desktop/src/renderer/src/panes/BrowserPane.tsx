import React, { useState, useCallback, useRef, useEffect } from 'react';
import { useConfig } from '../hooks/useConfig';
import { useTheme } from '../hooks/useTheme';
import { webviewThemeCSS, webviewThemeJS } from '../lib/webviewTheme';
import { webviewSettingsJS } from '../lib/webviewSettings';

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

const BrowserPane: React.FC<BrowserPaneProps> = ({ paneId, title, isActive, initialUrl, appMode, hibernated, onUrlChange, pluginId }) => {
  const { config } = useConfig();
  const browserCfg = config.browser ?? { homepage: 'https://google.com', bookmarks: [] };

  const [url, setUrl] = useState<string>(initialUrl || browserCfg.homepage || 'https://google.com');
  const [loading, setLoading] = useState(false);
  const [canGoBack, setCanGoBack] = useState(false);
  const [canGoForward, setCanGoForward] = useState(false);
  const [syncing, setSyncing] = useState(false);
  /** Transient toolbar status for the cookie sync (replaces the old alert()). */
  const [syncMsg, setSyncMsg] = useState<{ text: string; isError: boolean } | null>(null);
  const syncMsgTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [urlFocused, setUrlFocused] = useState(false);
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
    } catch { /* webview mid-navigation — re-applied on next dom-ready */ }
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
  useEffect(() => () => {
    if (syncMsgTimerRef.current) clearTimeout(syncMsgTimerRef.current);
  }, []);

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
        fontSize: '12px',
      }}
    >
      {/* URL bar + bookmarks — hidden in app mode */}
      {!appMode && (<>
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
          <NavButton onClick={handleBack} title="Back" disabled={!canGoBack}><IconBack /></NavButton>
          <NavButton onClick={handleForward} title="Forward" disabled={!canGoForward}><IconForward /></NavButton>
          <NavButton
            onClick={loading ? handleStopLoad : handleRefresh}
            title={loading ? 'Stop loading' : 'Refresh'}
          >
            {loading ? <IconStop /> : <IconRefresh />}
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
              onFocus={(e) => { setUrlFocused(true); e.currentTarget.select(); }}
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
                fontSize: '0.62rem',
                fontWeight: 600,
                color: syncMsg.isError ? 'var(--wks-error)' : 'var(--wks-text-muted)',
                animation: 'wks-fade-in 0.2s ease',
              }}
            >
              {syncMsg.text}
            </span>
          )}

          <ToolbarSep />
          <NavButton onClick={handleOpenExternal} title="Open in system browser"><IconExternal /></NavButton>
          <NavButton
            onClick={handleSyncChromeCookies}
            title="Sync sign-in cookies from Chrome (fixes stubborn OAuth flows)"
            disabled={syncing}
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
                  fontSize: '0.65rem',
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
      </>)}

      {/* App mode has no toolbar to host the progress bar — pin it to the pane top */}
      {appMode && loading && (
        <div className="wks-browser-progress" style={{ top: 0, bottom: 'auto' }} />
      )}

      {/* Content area */}
      {hibernated ? (
        <div style={{
          flex: 1,
          display: 'flex',
          flexDirection: 'column',
          alignItems: 'center',
          justifyContent: 'center',
          backgroundColor: 'var(--wks-bg-base)',
          color: 'var(--wks-text-muted)',
          gap: '8px',
        }}>
          <span style={{ fontSize: '2rem', opacity: 0.5 }}>&#x1F4A4;</span>
          <span style={{ fontSize: '0.7rem' }}>Hibernated</span>
          <span style={{ fontSize: '0.6rem', fontFamily: 'var(--wks-font-mono)', color: 'var(--wks-text-faint)' }}>{url}</span>
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
      e.currentTarget.style.color = disabled ? 'var(--wks-text-disabled)' : 'var(--wks-text-secondary)';
    }}
  >
    <span style={{ display: 'flex', animation: spinning ? 'wks-spin 0.9s linear infinite' : undefined }}>
      {children}
    </span>
  </button>
);

/** Thin vertical rule between toolbar groups (matches the composer's separators). */
const ToolbarSep: React.FC = () => (
  <span aria-hidden style={{
    width: 1, height: 14, flexShrink: 0, margin: '0 4px',
    background: 'var(--wks-border-subtle)',
  }} />
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
  <svg {...iconProps}><path d="M10 3.5 5.5 8l4.5 4.5" /></svg>
);

const IconForward: React.FC = () => (
  <svg {...iconProps}><path d="M6 3.5 10.5 8 6 12.5" /></svg>
);

const IconRefresh: React.FC = () => (
  <svg {...iconProps}>
    <path d="M14 8a6 6 0 1 1-6-6c1.68 0 3.29.67 4.49 1.83L14 5.33" />
    <polyline points="14 2 14 5.33 10.67 5.33" />
  </svg>
);

const IconStop: React.FC = () => (
  <svg {...iconProps}><path d="m4.5 4.5 7 7M11.5 4.5l-7 7" /></svg>
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
    {open
      ? <path d="M5.5 7V5a2.5 2.5 0 0 1 4.9-.7" />
      : <path d="M5.5 7V5a2.5 2.5 0 0 1 5 0v2" />}
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
