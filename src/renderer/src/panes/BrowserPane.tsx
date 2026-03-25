import React, { useState, useCallback, useRef, useEffect } from 'react';
import { useConfig } from '../hooks/useConfig';

interface BrowserPaneProps {
  paneId: string;
  title: string;
  isActive: boolean;
  initialUrl?: string;
  appMode?: boolean;
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

const BrowserPane: React.FC<BrowserPaneProps> = ({ paneId, title, isActive, initialUrl, appMode, onUrlChange }) => {
  const { config } = useConfig();
  const browserCfg = config.browser ?? { homepage: 'https://google.com', bookmarks: [] };

  const [url, setUrl] = useState<string>(initialUrl || browserCfg.homepage || 'https://google.com');
  const [loading, setLoading] = useState(false);
  const [canGoBack, setCanGoBack] = useState(false);
  const [canGoForward, setCanGoForward] = useState(false);
  const webviewRef = useRef<HTMLElement | null>(null);
  const readyRef = useRef(false);
  const onUrlChangeRef = useRef(onUrlChange);
  onUrlChangeRef.current = onUrlChange;

  // Attach webview event listeners once the element is ready
  useEffect(() => {
    const wv = webviewRef.current as any;
    if (!wv) return;

    const handleDomReady = () => {
      readyRef.current = true;
    };

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

      {/* Webview content area — real embedded browser */}
      {/* partition=persist:browser gives a persistent session (cookies survive restarts) */}
      {/* allowpopups needed for OAuth login flows that open popups */}
      <webview
        ref={webviewRef as any}
        src={startUrl}
        style={{
          flex: 1,
          width: '100%',
          border: 'none',
        }}
        // @ts-ignore — webview attributes not in React types
        partition="persist:browser"
        // @ts-ignore
        allowpopups="true"
      />
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
