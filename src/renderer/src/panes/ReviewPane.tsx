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
  /** Label for the inline action button ("Stage" / "Unstage"). */
  actionLabel: string;
  onAction: () => void;
  busy: boolean;
}> = ({ file, code, active, onClick, actionLabel, onAction, busy }) => {
  const meta = codeMeta(code);
  const label = file.orig_path ? `${file.orig_path} → ${file.path}` : file.path;
  return (
    <div
      onClick={onClick}
      title={label}
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 8,
        width: '100%',
        padding: '4px 10px',
        boxSizing: 'border-box',
        background: active ? 'var(--wks-bg-hover)' : 'transparent',
        color: active ? colors.textBright : colors.text,
        cursor: 'pointer',
        fontSize: '0.74rem',
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
          flex: 1,
          minWidth: 0,
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
          direction: 'rtl',
        }}
      >
        {label}
      </span>
      <button
        onClick={(e) => {
          e.stopPropagation();
          onAction();
        }}
        disabled={busy}
        title={actionLabel}
        style={{
          flexShrink: 0,
          padding: '1px 7px',
          borderRadius: 5,
          border: `1px solid ${colors.borderSubtle}`,
          background: 'transparent',
          color: colors.text,
          cursor: busy ? 'default' : 'pointer',
          fontSize: '0.64rem',
          fontFamily: 'inherit',
          opacity: busy ? 0.5 : 1,
        }}
      >
        {actionLabel}
      </button>
    </div>
  );
};

const SectionHeader: React.FC<{
  label: string;
  count: number;
  /** Optional right-aligned bulk action (e.g. "Stage all"). */
  action?: { label: string; onClick: () => void; busy: boolean };
}> = ({ label, count, action }) => (
  <div
    style={{
      display: 'flex',
      alignItems: 'center',
      justifyContent: 'space-between',
      gap: 8,
      padding: '8px 10px 4px',
      fontSize: '0.62rem',
      fontWeight: 700,
      letterSpacing: '0.06em',
      textTransform: 'uppercase',
      color: colors.muted,
    }}
  >
    <span>
      {label} {count > 0 && <span style={{ opacity: 0.7 }}>({count})</span>}
    </span>
    {action && (
      <button
        onClick={action.onClick}
        disabled={action.busy}
        style={{
          padding: '1px 7px',
          borderRadius: 5,
          border: `1px solid ${colors.borderSubtle}`,
          background: 'transparent',
          color: colors.text,
          cursor: action.busy ? 'default' : 'pointer',
          fontSize: '0.6rem',
          fontFamily: 'inherit',
          fontWeight: 600,
          letterSpacing: 0,
          textTransform: 'none',
          opacity: action.busy ? 0.5 : 1,
        }}
      >
        {action.label}
      </button>
    )}
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
  const [busy, setBusy] = useState(false);
  const [commitMsg, setCommitMsg] = useState('');

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

  // Run a mutating git action, then refresh status. Errors surface in the
  // banner; the action button stays disabled (busy) until it settles.
  const runAction = useCallback(
    async (fn: (dir: string) => Promise<unknown>) => {
      if (!cwd) return;
      setBusy(true);
      setError('');
      try {
        await fn(cwd);
        await refresh();
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err));
      } finally {
        setBusy(false);
      }
    },
    [cwd, refresh],
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
          <div style={{ display: 'flex', gap: 6, flexShrink: 0 }}>
            <button
              onClick={() => void runAction((dir) => git.push(dir))}
              disabled={busy}
              title="git push"
              style={{
                padding: '3px 10px',
                borderRadius: 6,
                border: `1px solid ${colors.borderSubtle}`,
                background: 'transparent',
                color: colors.text,
                cursor: busy ? 'default' : 'pointer',
                fontSize: '0.68rem',
                fontFamily: 'inherit',
              }}
            >
              {busy ? '…' : 'Push'}
            </button>
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
              }}
            >
              {loadingStatus ? '…' : 'Refresh'}
            </button>
          </div>
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

          {staged.length > 0 && (
            <SectionHeader
              label="Staged"
              count={staged.length}
              action={{
                label: 'Unstage all',
                busy,
                onClick: () => void runAction((dir) => git.unstage(dir)),
              }}
            />
          )}
          {staged.map((f) => {
            const sel: Selection = { path: f.path, staged: true, untracked: false };
            return (
              <FileRow
                key={`s:${f.path}`}
                file={f}
                code={f.staged}
                active={selection != null && selKey(selection) === selKey(sel)}
                onClick={() => void select(sel)}
                actionLabel="Unstage"
                onAction={() => void runAction((dir) => git.unstage(dir, f.path))}
                busy={busy}
              />
            );
          })}

          {unstaged.length > 0 && (
            <SectionHeader
              label="Changes"
              count={unstaged.length}
              action={{
                label: 'Stage all',
                busy,
                onClick: () => void runAction((dir) => git.stage(dir)),
              }}
            />
          )}
          {unstaged.map((f) => {
            const sel: Selection = { path: f.path, staged: false, untracked: false };
            return (
              <FileRow
                key={`w:${f.path}`}
                file={f}
                code={f.unstaged}
                active={selection != null && selKey(selection) === selKey(sel)}
                onClick={() => void select(sel)}
                actionLabel="Stage"
                onAction={() => void runAction((dir) => git.stage(dir, f.path))}
                busy={busy}
              />
            );
          })}

          {untracked.length > 0 && (
            <SectionHeader
              label="Untracked"
              count={untracked.length}
              // "Stage all" is `git add -A` (everything), so only offer it here
              // when there are no tracked changes — otherwise the Changes header
              // already covers it and a second identical button would mislead.
              action={
                unstaged.length === 0
                  ? {
                      label: 'Stage all',
                      busy,
                      onClick: () => void runAction((dir) => git.stage(dir)),
                    }
                  : undefined
              }
            />
          )}
          {untracked.map((f) => {
            const sel: Selection = { path: f.path, staged: false, untracked: true };
            return (
              <FileRow
                key={`u:${f.path}`}
                file={f}
                code="?"
                active={selection != null && selection.untracked && selection.path === f.path}
                onClick={() => void select(sel)}
                actionLabel="Stage"
                onAction={() => void runAction((dir) => git.stage(dir, f.path))}
                busy={busy}
              />
            );
          })}
        </div>

        {/* Commit bar — enabled only when something is staged. */}
        <div
          style={{
            borderTop: `1px solid ${colors.borderSubtle}`,
            padding: 8,
            display: 'flex',
            flexDirection: 'column',
            gap: 6,
          }}
        >
          <textarea
            value={commitMsg}
            onChange={(e) => setCommitMsg(e.target.value)}
            onKeyDown={(e) => {
              // Cmd/Ctrl+Enter commits, matching the usual editor convention.
              if (e.key === 'Enter' && (e.metaKey || e.ctrlKey) && staged.length > 0 && commitMsg.trim()) {
                e.preventDefault();
                void runAction(async (dir) => {
                  await git.commit(dir, commitMsg);
                  setCommitMsg('');
                });
              }
            }}
            placeholder={staged.length > 0 ? 'Commit message… (⌘/Ctrl+Enter)' : 'Stage files to commit'}
            disabled={busy || staged.length === 0}
            rows={2}
            style={{
              resize: 'none',
              padding: '6px 8px',
              borderRadius: 6,
              border: `1px solid ${colors.borderSubtle}`,
              background: 'var(--wks-bg-input, transparent)',
              color: colors.text,
              fontSize: '0.72rem',
              fontFamily: 'inherit',
              boxSizing: 'border-box',
            }}
          />
          <button
            onClick={() =>
              void runAction(async (dir) => {
                await git.commit(dir, commitMsg);
                setCommitMsg('');
              })
            }
            disabled={busy || staged.length === 0 || !commitMsg.trim()}
            style={{
              padding: '5px 10px',
              borderRadius: 6,
              border: `1px solid ${colors.borderSubtle}`,
              background: staged.length > 0 && commitMsg.trim() ? colors.accent : 'transparent',
              color: staged.length > 0 && commitMsg.trim() ? colors.textBright : colors.muted,
              cursor: busy || staged.length === 0 || !commitMsg.trim() ? 'default' : 'pointer',
              fontSize: '0.72rem',
              fontFamily: 'inherit',
              fontWeight: 600,
            }}
          >
            {`Commit${staged.length > 0 ? ` ${staged.length} file${staged.length > 1 ? 's' : ''}` : ''}`}
          </button>
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
