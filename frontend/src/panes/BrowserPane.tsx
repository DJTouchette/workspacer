import React, { useState, useCallback } from 'react';
import { useConfig } from '../hooks/useConfig';
// @ts-ignore — bindings not yet generated
import { OpenBrowser, CloseBrowser } from '../lib/browserApi';

interface BrowserPaneProps {
  paneId: string;
  title: string;
  isActive: boolean;
}

interface Bookmark {
  name: string;
  url: string;
}

const BrowserPane: React.FC<BrowserPaneProps> = ({ paneId, title, isActive }) => {
  const { config } = useConfig();
  const browserCfg = (config as any).browser ?? { homepage: 'https://google.com', bookmarks: [] };

  const [url, setUrl] = useState<string>(browserCfg.homepage || 'https://google.com');
  const [activeUrl, setActiveUrl] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const handleOpen = useCallback(() => {
    if (!url.trim()) return;
    let normalizedUrl = url.trim();
    if (!/^https?:\/\//i.test(normalizedUrl)) {
      normalizedUrl = 'https://' + normalizedUrl;
    }
    setUrl(normalizedUrl);
    setError(null);
    OpenBrowser(paneId, normalizedUrl)
      .then(() => {
        setActiveUrl(normalizedUrl);
      })
      .catch((err: any) => {
        setError(String(err));
      });
  }, [paneId, url]);

  const handleClose = useCallback(() => {
    CloseBrowser(paneId)
      .then(() => {
        setActiveUrl(null);
        setError(null);
      })
      .catch((err: any) => {
        setError(String(err));
      });
  }, [paneId]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === 'Enter') {
        e.preventDefault();
        handleOpen();
      }
    },
    [handleOpen],
  );

  const handleOpenExternal = useCallback(() => {
    if (!url.trim()) return;
    let normalizedUrl = url.trim();
    if (!/^https?:\/\//i.test(normalizedUrl)) {
      normalizedUrl = 'https://' + normalizedUrl;
    }
    window.open(normalizedUrl, '_blank');
  }, [url]);

  const handleBookmarkClick = useCallback(
    (bookmark: Bookmark) => {
      setUrl(bookmark.url);
      setError(null);
      OpenBrowser(paneId, bookmark.url)
        .then(() => {
          setActiveUrl(bookmark.url);
        })
        .catch((err: any) => {
          setError(String(err));
        });
    },
    [paneId],
  );

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
        transition: 'none',
      }}
    >
      {/* URL bar */}
      <div
        style={{
          display: 'flex',
          flexDirection: 'row',
          alignItems: 'center',
          gap: '4px',
          padding: '4px 6px',
          backgroundColor: '#1a1a1e',
          borderBottom: '1px solid #2a2a30',
          transition: 'none',
        }}
      >
        <input
          type="text"
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder="Enter URL..."
          spellCheck={false}
          style={{
            flex: 1,
            height: '28px',
            padding: '0 8px',
            fontSize: '12px',
            fontFamily: 'JetBrainsMono NF, JetBrainsMono Nerd Font, monospace',
            backgroundColor: '#0e0e10',
            color: '#e4e4e7',
            border: '1px solid #2a2a30',
            borderRadius: '3px',
            outline: 'none',
            transition: 'none',
          }}
          onFocus={(e) => {
            e.currentTarget.style.borderColor = '#60a5fa';
          }}
          onBlur={(e) => {
            e.currentTarget.style.borderColor = '#2a2a30';
          }}
        />
        <button
          onClick={handleOpen}
          style={{
            height: '28px',
            padding: '0 10px',
            fontSize: '11px',
            fontWeight: 600,
            backgroundColor: '#2563eb',
            color: '#e4e4e7',
            border: 'none',
            borderRadius: '3px',
            cursor: 'pointer',
            transition: 'none',
            whiteSpace: 'nowrap',
          }}
          onMouseEnter={(e) => {
            e.currentTarget.style.backgroundColor = '#3b82f6';
          }}
          onMouseLeave={(e) => {
            e.currentTarget.style.backgroundColor = '#2563eb';
          }}
        >
          Go
        </button>
        <button
          onClick={handleOpenExternal}
          title="Open in system browser"
          style={{
            height: '28px',
            padding: '0 8px',
            fontSize: '11px',
            backgroundColor: '#27272a',
            color: '#a1a1aa',
            border: '1px solid #3a3a40',
            borderRadius: '3px',
            cursor: 'pointer',
            transition: 'none',
            whiteSpace: 'nowrap',
          }}
          onMouseEnter={(e) => {
            e.currentTarget.style.backgroundColor = '#3a3a40';
          }}
          onMouseLeave={(e) => {
            e.currentTarget.style.backgroundColor = '#27272a';
          }}
        >
          External
        </button>
      </div>

      {/* Bookmarks bar */}
      {bookmarks.length > 0 && (
        <div
          style={{
            display: 'flex',
            flexDirection: 'row',
            flexWrap: 'wrap',
            gap: '3px',
            padding: '3px 6px',
            backgroundColor: '#16161a',
            borderBottom: '1px solid #2a2a30',
            transition: 'none',
          }}
        >
          {bookmarks.map((bm, i) => (
            <button
              key={i}
              onClick={() => handleBookmarkClick(bm)}
              title={bm.url}
              style={{
                height: '22px',
                padding: '0 8px',
                fontSize: '11px',
                backgroundColor: '#1e1e22',
                color: '#a0b4e6',
                border: '1px solid #2a2a30',
                borderRadius: '2px',
                cursor: 'pointer',
                transition: 'none',
                whiteSpace: 'nowrap',
              }}
              onMouseEnter={(e) => {
                e.currentTarget.style.backgroundColor = '#2a2a30';
                e.currentTarget.style.color = '#e4e4e7';
              }}
              onMouseLeave={(e) => {
                e.currentTarget.style.backgroundColor = '#1e1e22';
                e.currentTarget.style.color = '#a0b4e6';
              }}
            >
              {bm.name}
            </button>
          ))}
        </div>
      )}

      {/* Status area */}
      <div
        style={{
          flex: 1,
          display: 'flex',
          flexDirection: 'column',
          alignItems: 'center',
          justifyContent: 'center',
          gap: '8px',
          padding: '16px',
          transition: 'none',
        }}
      >
        {error && (
          <div
            style={{
              fontSize: '11px',
              color: '#f87171',
              backgroundColor: '#1e1012',
              border: '1px solid #7f1d1d',
              borderRadius: '3px',
              padding: '4px 10px',
              maxWidth: '400px',
              wordBreak: 'break-all',
              transition: 'none',
            }}
          >
            {error}
          </div>
        )}

        {activeUrl ? (
          <>
            <div
              style={{
                fontSize: '12px',
                color: '#a1a1aa',
                transition: 'none',
              }}
            >
              Browser open:{' '}
              <span style={{ color: '#60a5fa', wordBreak: 'break-all' }}>
                {activeUrl}
              </span>
            </div>
            <button
              onClick={handleClose}
              style={{
                height: '26px',
                padding: '0 14px',
                fontSize: '11px',
                fontWeight: 600,
                backgroundColor: '#7f1d1d',
                color: '#fca5a5',
                border: 'none',
                borderRadius: '3px',
                cursor: 'pointer',
                transition: 'none',
              }}
              onMouseEnter={(e) => {
                e.currentTarget.style.backgroundColor = '#991b1b';
              }}
              onMouseLeave={(e) => {
                e.currentTarget.style.backgroundColor = '#7f1d1d';
              }}
            >
              Close Browser
            </button>
          </>
        ) : (
          <div
            style={{
              fontSize: '12px',
              color: '#71717a',
              transition: 'none',
            }}
          >
            Enter a URL or click a bookmark
          </div>
        )}
      </div>
    </div>
  );
};

export default BrowserPane;
