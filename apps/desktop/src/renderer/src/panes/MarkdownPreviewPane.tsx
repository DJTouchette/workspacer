import React, { useCallback, useEffect, useState } from 'react';
import { Markdown } from '../components/markdown';
import { claudeColors as colors } from '../components/claude-shared';
import { RefreshCw } from '../components/icons';
import { requestOpenInEditor } from '../lib/editorBus';

/**
 * MarkdownPreviewPane — renders one .md file as a readable document using the
 * chat's markdown renderer. Opened from FileLink (left-click on a .md path in
 * a tool-call card) via previewBus → App → openMarkdownPreview. The file is
 * re-read on demand (refresh button) rather than watched — previews are for
 * reading a doc the agent just wrote, not live editing.
 */

const headerBtnStyle: React.CSSProperties = {
  display: 'inline-flex',
  alignItems: 'center',
  gap: 5,
  padding: '3px 10px',
  borderRadius: 'var(--wks-radius-sm)',
  border: '1px solid var(--wks-border-input)',
  background: 'transparent',
  color: 'var(--wks-text-primary)',
  cursor: 'pointer',
  fontSize: '0.7rem',
  fontFamily: 'inherit',
  fontWeight: 600,
  flexShrink: 0,
};

const MarkdownPreviewPane: React.FC<{
  title?: string;
  previewPath?: string;
  previewCwd?: string;
}> = ({ previewPath, previewCwd }) => {
  const [content, setContent] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const load = useCallback(async () => {
    if (!previewPath) return;
    setLoading(true);
    try {
      const res = await window.electronAPI.readFile(previewPath);
      setContent(res.contents);
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  }, [previewPath]);

  useEffect(() => {
    void load();
  }, [load]);

  const fileName = previewPath?.replace(/\\/g, '/').split('/').pop() ?? 'Markdown';

  return (
    <div
      style={{
        width: '100%',
        height: '100%',
        display: 'flex',
        flexDirection: 'column',
        background: 'var(--wks-bg-base)',
        color: 'var(--wks-text-primary)',
        overflow: 'hidden',
      }}
    >
      {/* Header: file name + path, refresh, open in editor */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 10,
          padding: '8px 14px',
          borderBottom: '1px solid var(--wks-border)',
          flexShrink: 0,
        }}
      >
        <span style={{ fontSize: '0.82rem', fontWeight: 700, flexShrink: 0 }}>{fileName}</span>
        {previewPath && (
          <span
            title={previewPath}
            style={{
              fontSize: '0.68rem',
              fontFamily: 'var(--wks-font-mono)',
              color: 'var(--wks-text-faint)',
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
              minWidth: 0,
            }}
          >
            {previewPath}
          </span>
        )}
        <div style={{ flex: 1 }} />
        <button style={headerBtnStyle} onClick={() => void load()} title="Reload from disk">
          <RefreshCw
            size={11}
            style={loading ? { animation: 'wks-spin 1s linear infinite' } : undefined}
          />
          Refresh
        </button>
        {previewPath && (
          <button
            style={headerBtnStyle}
            onClick={() => requestOpenInEditor({ path: previewPath, cwd: previewCwd })}
          >
            Open in editor
          </button>
        )}
      </div>

      {/* Document body */}
      <div style={{ flex: 1, overflowY: 'auto', minHeight: 0 }}>
        {!previewPath ? (
          <div style={{ padding: 24, color: 'var(--wks-text-muted)', fontSize: '0.8rem' }}>
            This preview lost its file target — close the pane and reopen it from a file link.
          </div>
        ) : error ? (
          <div style={{ padding: 24, fontSize: '0.8rem' }}>
            <div style={{ color: 'var(--wks-error)', fontWeight: 600, marginBottom: 6 }}>
              Couldn’t read {fileName}
            </div>
            <div
              style={{
                color: 'var(--wks-text-muted)',
                fontFamily: 'var(--wks-font-mono)',
                fontSize: '0.7rem',
                wordBreak: 'break-word',
              }}
            >
              {error}
            </div>
          </div>
        ) : content === null ? (
          <div style={{ padding: 24, color: 'var(--wks-text-muted)', fontSize: '0.8rem' }}>
            Loading…
          </div>
        ) : (
          <div
            style={{
              maxWidth: 840,
              margin: '0 auto',
              padding: '26px 36px 60px',
              fontSize: 'calc(0.84rem * var(--claude-gui-font-scale, 1))',
              lineHeight: 1.65,
              color: colors.text,
            }}
          >
            {content.trim() === '' ? (
              <span style={{ color: 'var(--wks-text-muted)' }}>(empty file)</span>
            ) : (
              <Markdown text={content} />
            )}
          </div>
        )}
      </div>
    </div>
  );
};

export default MarkdownPreviewPane;
