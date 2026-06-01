import React, { useState, useEffect, useCallback, useMemo } from 'react';
import { claudeColors as colors } from '../components/claude-shared';
import { GitClient, type GitStatus, type FileStatus } from '../lib/gitQueries';

interface ReviewPaneProps {
  paneId: string;
  title: string;
  isActive: boolean;
  /** Working directory to inspect — inherited from the active agent. */
  cwd?: string;
}

const git = new GitClient();

// ── Status code → presentation ──

/** Map a porcelain status char to a color + short label. */
function codeMeta(code: string): { color: string; label: string } {
  switch (code) {
    case 'M':
      return { color: colors.warning, label: 'M' };
    case 'A':
      return { color: colors.success, label: 'A' };
    case 'D':
      return { color: colors.error, label: 'D' };
    case 'R':
      return { color: colors.accent, label: 'R' };
    case 'C':
      return { color: colors.accent, label: 'C' };
    case 'U':
      return { color: colors.error, label: 'U' };
    case '?':
      return { color: colors.muted, label: '?' };
    default:
      return { color: colors.muted, label: code };
  }
}

/** Which file is currently selected, and which side of the diff to show. */
interface Selection {
  path: string;
  staged: boolean;
  /** Untracked files have no diff to render. */
  untracked: boolean;
}

function selKey(s: Selection): string {
  return `${s.staged ? 's' : 'w'}:${s.path}`;
}

// ── Diff rendering ──

function lineColor(line: string): string {
  if (line.startsWith('+++') || line.startsWith('---')) return colors.muted;
  if (line.startsWith('@@')) return colors.accent;
  if (line.startsWith('diff ') || line.startsWith('index ')) return colors.muted;
  if (line.startsWith('+')) return colors.success;
  if (line.startsWith('-')) return colors.error;
  return colors.text;
}

const DiffView: React.FC<{ text: string }> = ({ text }) => {
  const lines = useMemo(() => text.split('\n'), [text]);
  return (
    <pre
      style={{
        margin: 0,
        padding: '12px 14px',
        fontSize: '0.72rem',
        lineHeight: 1.5,
        fontFamily: 'var(--wks-font-mono, monospace)',
        whiteSpace: 'pre',
        overflow: 'auto',
        height: '100%',
        boxSizing: 'border-box',
      }}
    >
      {lines.map((line, i) => (
        <div key={i} style={{ color: lineColor(line) }}>
          {line || ' '}
        </div>
      ))}
    </pre>
  );
};

// ── File row ──

const FileRow: React.FC<{
  file: FileStatus;
  code: string;
  active: boolean;
  onClick: () => void;
}> = ({ file, code, active, onClick }) => {
  const meta = codeMeta(code);
  const label = file.orig_path ? `${file.orig_path} → ${file.path}` : file.path;
  return (
    <button
      onClick={onClick}
      title={label}
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 8,
        width: '100%',
        padding: '4px 10px',
        border: 'none',
        background: active ? 'var(--wks-bg-hover)' : 'transparent',
        color: active ? colors.textBright : colors.text,
        cursor: 'pointer',
        fontFamily: 'inherit',
        fontSize: '0.74rem',
        textAlign: 'left',
        borderLeft: `2px solid ${active ? colors.accent : 'transparent'}`,
      }}
    >
      <span
        style={{
          color: meta.color,
          fontWeight: 700,
          fontFamily: 'var(--wks-font-mono, monospace)',
          width: 12,
          flexShrink: 0,
        }}
      >
        {meta.label}
      </span>
      <span
        style={{
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
          direction: 'rtl',
        }}
      >
        {label}
      </span>
    </button>
  );
};

const SectionHeader: React.FC<{ label: string; count: number }> = ({ label, count }) => (
  <div
    style={{
      padding: '8px 10px 4px',
      fontSize: '0.62rem',
      fontWeight: 700,
      letterSpacing: '0.06em',
      textTransform: 'uppercase',
      color: colors.muted,
    }}
  >
    {label} {count > 0 && <span style={{ opacity: 0.7 }}>({count})</span>}
  </div>
);

// ── Pane ──

const ReviewPane: React.FC<ReviewPaneProps> = ({ cwd }) => {
  const [status, setStatus] = useState<GitStatus | null>(null);
  const [selection, setSelection] = useState<Selection | null>(null);
  const [diff, setDiff] = useState<string>('');
  const [error, setError] = useState<string>('');
  const [loadingStatus, setLoadingStatus] = useState(false);
  const [loadingDiff, setLoadingDiff] = useState(false);

  const refresh = useCallback(async () => {
    if (!cwd) return;
    setLoadingStatus(true);
    setError('');
    try {
      const s = await git.status(cwd);
      setStatus(s);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setStatus(null);
    } finally {
      setLoadingStatus(false);
    }
  }, [cwd]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // Group files into staged / unstaged / untracked sections.
  const { staged, unstaged, untracked } = useMemo(() => {
    const staged: FileStatus[] = [];
    const unstaged: FileStatus[] = [];
    const untracked: FileStatus[] = [];
    for (const f of status?.files ?? []) {
      if (f.staged === '?') {
        untracked.push(f);
        continue;
      }
      if (f.staged !== ' ') staged.push(f);
      if (f.unstaged !== ' ') unstaged.push(f);
    }
    return { staged, unstaged, untracked };
  }, [status]);

  const select = useCallback(
    async (sel: Selection) => {
      setSelection(sel);
      setDiff('');
      if (sel.untracked || !cwd) return;
      setLoadingDiff(true);
      try {
        const text = await git.diff(cwd, sel.path, sel.staged);
        setDiff(text);
      } catch (err) {
        setDiff(`# failed to load diff\n${err instanceof Error ? err.message : String(err)}`);
      } finally {
        setLoadingDiff(false);
      }
    },
    [cwd],
  );

  if (!cwd) {
    return (
      <Centered>
        <span style={{ fontSize: '1.5rem' }}>&#129518;</span>
        <span>No working directory for this pane.</span>
      </Centered>
    );
  }

  const totalChanges = staged.length + unstaged.length + untracked.length;

  return (
    <div
      style={{
        display: 'flex',
        height: '100%',
        backgroundColor: colors.bg,
        color: colors.text,
        fontFamily: 'inherit',
      }}
    >
      {/* Left: file list */}
      <div
        style={{
          width: 280,
          flexShrink: 0,
          borderRight: `1px solid ${colors.borderSubtle}`,
          display: 'flex',
          flexDirection: 'column',
          overflow: 'hidden',
        }}
      >
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            gap: 8,
            padding: '8px 10px',
            borderBottom: `1px solid ${colors.borderSubtle}`,
          }}
        >
          <span
            style={{
              fontSize: '0.74rem',
              fontWeight: 600,
              color: colors.textBright,
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
            }}
            title={cwd}
          >
            {status?.branch ? `⎇ ${status.branch}` : 'git'}
          </span>
          <button
            onClick={() => void refresh()}
            disabled={loadingStatus}
            style={{
              padding: '3px 10px',
              borderRadius: 6,
              border: `1px solid ${colors.borderSubtle}`,
              background: 'transparent',
              color: colors.text,
              cursor: loadingStatus ? 'default' : 'pointer',
              fontSize: '0.68rem',
              fontFamily: 'inherit',
              flexShrink: 0,
            }}
          >
            {loadingStatus ? '…' : 'Refresh'}
          </button>
        </div>

        <div style={{ overflowY: 'auto', flex: 1 }}>
          {error && (
            <div style={{ padding: '10px', color: colors.error, fontSize: '0.72rem' }}>{error}</div>
          )}
          {!error && totalChanges === 0 && !loadingStatus && (
            <div style={{ padding: '14px 10px', color: colors.muted, fontSize: '0.74rem' }}>
              Working tree clean.
            </div>
          )}

          {staged.length > 0 && <SectionHeader label="Staged" count={staged.length} />}
          {staged.map((f) => {
            const sel: Selection = { path: f.path, staged: true, untracked: false };
            return (
              <FileRow
                key={`s:${f.path}`}
                file={f}
                code={f.staged}
                active={selection != null && selKey(selection) === selKey(sel)}
                onClick={() => void select(sel)}
              />
            );
          })}

          {unstaged.length > 0 && <SectionHeader label="Changes" count={unstaged.length} />}
          {unstaged.map((f) => {
            const sel: Selection = { path: f.path, staged: false, untracked: false };
            return (
              <FileRow
                key={`w:${f.path}`}
                file={f}
                code={f.unstaged}
                active={selection != null && selKey(selection) === selKey(sel)}
                onClick={() => void select(sel)}
              />
            );
          })}

          {untracked.length > 0 && <SectionHeader label="Untracked" count={untracked.length} />}
          {untracked.map((f) => {
            const sel: Selection = { path: f.path, staged: false, untracked: true };
            return (
              <FileRow
                key={`u:${f.path}`}
                file={f}
                code="?"
                active={selection != null && selection.untracked && selection.path === f.path}
                onClick={() => void select(sel)}
              />
            );
          })}
        </div>
      </div>

      {/* Right: diff */}
      <div style={{ flex: 1, overflow: 'hidden', minWidth: 0 }}>
        {!selection ? (
          <Centered>
            <span style={{ color: colors.muted }}>Select a file to view its diff.</span>
          </Centered>
        ) : selection.untracked ? (
          <Centered>
            <span style={{ color: colors.muted }}>
              {selection.path} is untracked — no diff available.
            </span>
          </Centered>
        ) : loadingDiff ? (
          <Centered>
            <span style={{ color: colors.muted }}>Loading diff…</span>
          </Centered>
        ) : diff.trim() === '' ? (
          <Centered>
            <span style={{ color: colors.muted }}>No textual diff (binary or no changes).</span>
          </Centered>
        ) : (
          <DiffView text={diff} />
        )}
      </div>
    </div>
  );
};

const Centered: React.FC<{ children: React.ReactNode }> = ({ children }) => (
  <div
    style={{
      display: 'flex',
      flexDirection: 'column',
      alignItems: 'center',
      justifyContent: 'center',
      height: '100%',
      gap: 10,
      color: colors.text,
      fontSize: '0.8rem',
      padding: 20,
      textAlign: 'center',
    }}
  >
    {children}
  </div>
);

export default ReviewPane;
