import React, { useState, useCallback, useRef } from 'react';
import { useConfig } from '../hooks/useConfig';
import { FetchPage } from '../lib/browserApi';

interface BrowserPaneProps {
  paneId: string;
  title: string;
  isActive: boolean;
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

const BrowserPane: React.FC<BrowserPaneProps> = ({ paneId, title, isActive }) => {
  const { config } = useConfig();
  const browserCfg = (config as any).browser ?? { homepage: 'https://google.com', bookmarks: [] };

  const [url, setUrl] = useState<string>(browserCfg.homepage || 'https://google.com');
  const [pageHtml, setPageHtml] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [currentUrl, setCurrentUrl] = useState<string | null>(null);
  const contentRef = useRef<HTMLDivElement>(null);

  const navigate = useCallback((targetUrl: string) => {
    const normalized = normalizeUrl(targetUrl);
    if (!normalized) return;
    setUrl(normalized);
    setCurrentUrl(normalized);
    setLoading(true);
    setError(null);
    setPageHtml(null);

    FetchPage(normalized)
      .then((html) => {
        setPageHtml(html);
        setLoading(false);
        // Scroll content to top
        if (contentRef.current) contentRef.current.scrollTop = 0;
      })
      .catch((err) => {
        setError(String(err));
        setLoading(false);
      });
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
    if (currentUrl) navigate(currentUrl);
  }, [currentUrl, navigate]);

  const handleOpenExternal = useCallback(() => {
    const normalized = normalizeUrl(url);
    if (normalized) window.open(normalized, '_blank');
  }, [url]);

  // Intercept link clicks inside the rendered content
  const handleContentClick = useCallback((e: React.MouseEvent) => {
    const target = e.target as HTMLElement;
    const anchor = target.closest('a');
    if (anchor && anchor.href) {
      e.preventDefault();
      const href = anchor.href;
      // Skip javascript: and # links
      if (href.startsWith('javascript:') || href === '#') return;
      if (href.startsWith('#')) return;
      setUrl(href);
      navigate(href);
    }
  }, [navigate]);

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
      {/* URL bar */}
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

      {/* Bookmarks bar */}
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

      {/* Content area */}
      <div
        ref={contentRef}
        onClick={handleContentClick}
        style={{
          flex: 1,
          overflow: 'auto',
          backgroundColor: '#fff',
          color: '#1a1a1a',
        }}
      >
        {loading && (
          <div style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            height: '100%',
            color: '#71717a',
            backgroundColor: '#121214',
            fontSize: '12px',
          }}>
            Loading...
          </div>
        )}

        {error && (
          <div style={{
            display: 'flex',
            flexDirection: 'column',
            alignItems: 'center',
            justifyContent: 'center',
            height: '100%',
            backgroundColor: '#121214',
            gap: '8px',
            padding: '16px',
          }}>
            <div style={{ fontSize: '11px', color: '#f87171' }}>{error}</div>
            <button
              onClick={handleOpenExternal}
              style={{
                ...navBtnStyle,
                width: 'auto',
                padding: '0 12px',
                height: '28px',
                fontSize: '11px',
              }}
            >
              Open in system browser instead
            </button>
          </div>
        )}

        {pageHtml && !loading && (
          <div
            dangerouslySetInnerHTML={{ __html: pageHtml }}
            style={{ padding: '0', minHeight: '100%' }}
          />
        )}

        {!pageHtml && !loading && !error && (
          <div style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            height: '100%',
            color: '#71717a',
            backgroundColor: '#121214',
            fontSize: '12px',
          }}>
            Enter a URL or click a bookmark
          </div>
        )}
      </div>
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
