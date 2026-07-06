import React, { useState, useEffect, useCallback, useMemo, useRef } from 'react';
import { RefreshCw, ArrowUp, CheckCircle2, Copy, Check, FileX2 } from 'lucide-react';
import { IconBranch, IconCommit } from '../components/wksIcons';
import { claudeColors as colors } from '../components/claude-shared';
import { GitClient, type GitStatus, type NumstatEntry } from '../lib/gitQueries';
import FileTree, { StatusChip, type TreeEntry } from '../components/review/FileTree';
import DiffViewer from '../components/review/DiffViewer';
import { ensureReviewStyles } from '../components/review/reviewStyles';
import { parseUnifiedDiff } from '../lib/diff/parseDiff';
import { REVIEW_OPEN_FILE_EVENT } from '../lib/reviewBus';
import { requestOpenInEditor } from '../lib/editorBus';

interface ReviewPaneProps {
  paneId: string;
  title: string;
  isActive: boolean;
  /** Working directory to inspect — inherited from the active agent. */
  cwd?: string;
}

const git = new GitClient();

/** Diffs past this many chars need an explicit click to render. */
const LARGE_DIFF_CHARS = 1_500_000;

/** Which file is currently selected, and which side of the diff to show. */
interface Selection {
  path: string;
  staged: boolean;
  /** Untracked files render as an all-added diff via --no-index. */
  untracked: boolean;
}

function selKey(s: Selection): string {
  return `${s.untracked ? 'u' : s.staged ? 's' : 'w'}:${s.path}`;
}

// ── Small presentational pieces ──

/** GitHub-style five-block diffstat (green/red/neutral). */
const DiffStatBlocks: React.FC<{ added: number; deleted: number }> = ({ added, deleted }) => {
  const total = added + deleted;
  if (total === 0) return null;
  const green = Math.round((added / total) * 5);
  const red = Math.min(5 - green, Math.ceil((deleted / total) * 5));
  const blocks = [
    ...Array(green).fill(colors.success),
    ...Array(red).fill(colors.error),
    ...Array(Math.max(0, 5 - green - red)).fill('var(--wks-claude-border-subtle)'),
  ];
  return (
    <span style={{ display: 'inline-flex', gap: 2, flexShrink: 0 }} aria-hidden>
      {blocks.map((bg, i) => (
        <span key={i} style={{ width: 7, height: 7, borderRadius: 2, background: bg }} />
      ))}
    </span>
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
      padding: '10px 12px 4px',
      fontSize: '0.6rem',
      fontWeight: 700,
      letterSpacing: '0.08em',
      textTransform: 'uppercase',
      color: colors.muted,
    }}
  >
    <span style={{ display: 'inline-flex', alignItems: 'center', gap: 6 }}>
      {label}
      {count > 0 && (
        <span
          style={{
            background: 'var(--wks-bg-hover)',
            borderRadius: 'var(--wks-radius-md)',
            padding: '0 6px',
            fontSize: '0.58rem',
            lineHeight: '14px',
            letterSpacing: 0,
          }}
        >
          {count}
        </span>
      )}
    </span>
    {action && (
      <button
        onClick={action.onClick}
        disabled={action.busy}
        style={{
          padding: '1px 7px',
          borderRadius: 'var(--wks-radius-sm)',
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

/** Shimmer placeholder shown while a diff loads. */
const DiffSkeleton: React.FC = () => {
  const widths = [62, 38, 75, 51, 84, 30, 68, 45, 57, 72, 40, 66];
  return (
    <div style={{ padding: '16px 18px', display: 'flex', flexDirection: 'column', gap: 9 }}>
      {widths.map((w, i) => (
        <div key={i} className="wks-review-skeleton" style={{ height: 11, width: `${w}%` }} />
      ))}
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
      fontSize: '0.78rem',
      padding: 20,
      textAlign: 'center',
    }}
  >
    {children}
  </div>
);

/** Left-to-Right mark (U+200E). Wrapping path text in these pins it to LTR
 *  inside the RTL (left-ellipsis) container, so slashes don't get
 *  bidi-reordered (otherwise a trailing "/" jumps to the far end). */
const LRM = String.fromCharCode(0x200e);

/** File path with dimmed directories and a bright basename, truncating the
 * directory part from the left when space runs out. */
const PathBreadcrumb: React.FC<{ path: string }> = ({ path }) => {
  const i = path.lastIndexOf('/');
  const dir = i >= 0 ? path.slice(0, i + 1) : '';
  const base = i >= 0 ? path.slice(i + 1) : path;
  return (
    <span
      title={path}
      style={{
        display: 'flex',
        alignItems: 'baseline',
        minWidth: 0,
        fontFamily: 'var(--wks-font-mono, monospace)',
        fontSize: '0.74rem',
      }}
    >
      {dir && (
        <span
          style={{
            color: colors.muted,
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
            direction: 'rtl',
            flexShrink: 1,
            minWidth: 0,
          }}
        >
          {/* The container is RTL so the ellipsis clips on the LEFT (keeping the
              meaningful tail of the path). Wrap the text in Left-to-Right marks
              (U+200E) so its slashes don't get bidi-reordered — without these,
              the trailing "/" flips to the far end and jams the basename. */}
          {LRM + dir + LRM}
        </span>
      )}
      <span style={{ color: colors.textBright, fontWeight: 600, flexShrink: 0 }}>{base}</span>
    </span>
  );
};

// ── Pane ──

const ReviewPane: React.FC<ReviewPaneProps> = ({ cwd, isActive }) => {
  ensureReviewStyles();
  const [status, setStatus] = useState<GitStatus | null>(null);
  const [stats, setStats] = useState<{
    staged: Map<string, NumstatEntry>;
    unstaged: Map<string, NumstatEntry>;
  }>({ staged: new Map(), unstaged: new Map() });
  const [selection, setSelection] = useState<Selection | null>(null);
  const [diffText, setDiffText] = useState<string>('');
  const [diffError, setDiffError] = useState<string>('');
  const [error, setError] = useState<string>('');
  const [loadingStatus, setLoadingStatus] = useState(false);
  const [loadingDiff, setLoadingDiff] = useState(false);
  const [busy, setBusy] = useState(false);
  const [commitMsg, setCommitMsg] = useState('');
  const [copied, setCopied] = useState(false);
  /** Selection keys the user explicitly asked to render despite their size. */
  const [forcedLarge, setForcedLarge] = useState<ReadonlySet<string>>(new Set());

  // Diff cache, invalidated wholesale on every refresh (any git action can
  // shift line numbers in unrelated hunks).
  const diffCache = useRef<Map<string, string>>(new Map());

  const refresh = useCallback(async () => {
    if (!cwd) return;
    setLoadingStatus(true);
    setError('');
    diffCache.current.clear();
    try {
      // Counts are decoration — don't let a numstat hiccup take down status.
      const [s, unstagedStats, stagedStats] = await Promise.all([
        git.status(cwd),
        git.numstat(cwd, false).catch(() => [] as NumstatEntry[]),
        git.numstat(cwd, true).catch(() => [] as NumstatEntry[]),
      ]);
      setStatus(s);
      setStats({
        staged: new Map(stagedStats.map((e) => [e.path, e])),
        unstaged: new Map(unstagedStats.map((e) => [e.path, e])),
      });
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

  // Re-sync whenever the pane regains focus — the agent may have changed
  // files in the meantime. Skips the mount, which the effect above covers.
  const mounted = useRef(false);
  useEffect(() => {
    if (!mounted.current) {
      mounted.current = true;
      return;
    }
    if (isActive) void refresh();
  }, [isActive, refresh]);

  // Group files into staged / unstaged / untracked tree entries.
  const { staged, unstaged, untracked } = useMemo(() => {
    const staged: TreeEntry[] = [];
    const unstaged: TreeEntry[] = [];
    const untracked: TreeEntry[] = [];
    for (const f of status?.files ?? []) {
      if (f.staged === '?') {
        untracked.push({ file: f, code: 'A', key: `u:${f.path}` });
        continue;
      }
      if (f.staged !== ' ') staged.push({ file: f, code: f.staged, key: `s:${f.path}` });
      if (f.unstaged !== ' ') unstaged.push({ file: f, code: f.unstaged, key: `w:${f.path}` });
    }
    return { staged, unstaged, untracked };
  }, [status]);

  const totals = useMemo(() => {
    let added = 0;
    let deleted = 0;
    for (const map of [stats.staged, stats.unstaged]) {
      for (const e of map.values()) {
        added += e.added ?? 0;
        deleted += e.deleted ?? 0;
      }
    }
    return { added, deleted };
  }, [stats]);

  const select = useCallback(
    async (sel: Selection) => {
      setSelection(sel);
      setDiffError('');
      if (!cwd) return;
      const key = selKey(sel);
      const cached = diffCache.current.get(key);
      if (cached !== undefined) {
        setDiffText(cached);
        return;
      }
      setDiffText('');
      setLoadingDiff(true);
      try {
        const text = await git.diff(cwd, sel.path, sel.staged, sel.untracked);
        diffCache.current.set(key, text);
        setDiffText(text);
      } catch (err) {
        setDiffError(err instanceof Error ? err.message : String(err));
      } finally {
        setLoadingDiff(false);
      }
    },
    [cwd],
  );

  // ── Open a specific file from elsewhere (e.g. the Claude pane's inspector
  // rail). The target path is resolved against git status, so callers don't
  // need to know whether the change is staged/unstaged/untracked. ──
  const pendingPathRef = useRef<string | null>(null);
  const [openNonce, setOpenNonce] = useState(0);
  useEffect(() => {
    const norm = (p: string) => p.replace(/\\/g, '/').replace(/\/+$/, '').toLowerCase();
    const handler = (e: Event) => {
      const d = (e as CustomEvent).detail as { path?: string; cwd?: string } | undefined;
      if (!d?.path) return;
      if (d.cwd && cwd && norm(d.cwd) !== norm(cwd)) return; // not this repo's pane
      pendingPathRef.current = d.path;
      setOpenNonce((n) => n + 1);
      void refresh(); // the file may be new since our last status load
    };
    window.addEventListener(REVIEW_OPEN_FILE_EVENT, handler);
    return () => window.removeEventListener(REVIEW_OPEN_FILE_EVENT, handler);
  }, [cwd, refresh]);

  // Resolve the pending path against status once it's available. Claude emits
  // absolute paths while git status is repo-relative, so match by suffix or
  // basename. Prefer the unstaged working change, else staged, else untracked.
  useEffect(() => {
    const target = pendingPathRef.current;
    if (!target || !status) return;
    const norm = (p: string) => p.replace(/\\/g, '/');
    const t = norm(target);
    const base = t.split('/').pop();
    const match = (status.files ?? []).find((f) => {
      const fp = norm(f.path);
      return t === fp || t.endsWith('/' + fp) || fp.split('/').pop() === base;
    });
    pendingPathRef.current = null;
    if (!match) return;
    const sel: Selection =
      match.staged === '?'
        ? { path: match.path, staged: false, untracked: true }
        : match.unstaged !== ' '
          ? { path: match.path, staged: false, untracked: false }
          : { path: match.path, staged: true, untracked: false };
    void select(sel);
  }, [status, openNonce, select]);

  // Keep the selection valid across refreshes: drop it if its file is gone,
  // and default to the first changed file so the pane never opens empty.
  useEffect(() => {
    if (!status) return;
    // Don't steal the selection from a pending open-file request.
    if (pendingPathRef.current) return;
    const all = [...unstaged, ...staged, ...untracked];
    const currentKey = selection ? selKey(selection) : null;
    if (currentKey && all.some((e) => e.key === currentKey)) {
      // Same file may have new content after a git action — reload it.
      void select(selection!);
      return;
    }
    const first = all[0];
    if (first) {
      void select({
        path: first.file.path,
        staged: first.key.startsWith('s:'),
        untracked: first.key.startsWith('u:'),
      });
    } else {
      setSelection(null);
      setDiffText('');
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [status]);

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

  const commit = useCallback(() => {
    void runAction(async (dir) => {
      await git.commit(dir, commitMsg);
      setCommitMsg('');
    });
  }, [runAction, commitMsg]);

  const parsed = useMemo(() => {
    if (!diffText) return null;
    if (diffText.length > LARGE_DIFF_CHARS && selection && !forcedLarge.has(selKey(selection))) {
      return null;
    }
    return parseUnifiedDiff(diffText);
  }, [diffText, selection, forcedLarge]);

  if (!cwd) {
    return (
      <Centered>
        <FileX2 size={28} style={{ color: colors.muted, opacity: 0.7 }} />
        <span>No working directory for this pane.</span>
      </Centered>
    );
  }

  const totalChanges = staged.length + unstaged.length + untracked.length;
  const selectionStats = selection
    ? selection.untracked
      ? parsed
        ? { path: selection.path, added: parsed.additions, deleted: parsed.deletions }
        : undefined
      : (selection.staged ? stats.staged : stats.unstaged).get(selection.path)
    : undefined;
  const isLargeGated =
    diffText.length > LARGE_DIFF_CHARS && selection != null && !forcedLarge.has(selKey(selection));
  const canCommit = staged.length > 0 && commitMsg.trim().length > 0 && !busy;

  const treeProps = (section: 'staged' | 'unstaged' | 'untracked') => ({
    selectedKey: selection ? selKey(selection) : null,
    busy,
    stats: section === 'staged' ? stats.staged : stats.unstaged,
    onSelect: (e: TreeEntry) =>
      void select({
        path: e.file.path,
        staged: section === 'staged',
        untracked: section === 'untracked',
      }),
    actionLabel: section === 'staged' ? 'Unstage' : 'Stage',
    onAction: (e: TreeEntry) =>
      void runAction((dir) =>
        section === 'staged' ? git.unstage(dir, e.file.path) : git.stage(dir, e.file.path),
      ),
    onOpenInEditor: (e: TreeEntry) =>
      requestOpenInEditor({
        path: cwd ? `${cwd.replace(/[\\/]$/, '')}/${e.file.path}` : e.file.path,
        cwd,
      }),
  });

  return (
    <div
      style={{
        display: 'flex',
        flexDirection: 'column',
        height: '100%',
        backgroundColor: colors.bg,
        color: colors.text,
        fontFamily: 'inherit',
        padding: '14px 18px',
        boxSizing: 'border-box',
        gap: 12,
      }}
    >
      {/* Header — title + branch / change summary + actions (mockup "Reviewing changes"). */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          gap: 16,
          flexShrink: 0,
        }}
      >
        <div style={{ minWidth: 0 }}>
          <div style={{ fontSize: '0.95rem', fontWeight: 700, color: colors.textBright }}>
            Reviewing changes
          </div>
          <div
            style={{
              fontFamily: 'var(--wks-font-mono, monospace)',
              fontSize: '0.7rem',
              color: colors.muted,
              marginTop: 3,
              display: 'flex',
              alignItems: 'center',
              gap: 8,
              flexWrap: 'wrap',
              minWidth: 0,
            }}
          >
            <span
              style={{ display: 'inline-flex', alignItems: 'center', gap: 5, minWidth: 0 }}
              title={cwd}
            >
              <IconBranch
                size={12}
                strokeWidth={2.2}
                style={{ flexShrink: 0, color: colors.accent }}
                accent={colors.accent}
              />
              <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                {status?.branch ?? 'detached'}
              </span>
            </span>
            {totalChanges > 0 && (
              <>
                <span>
                  · {totalChanges} file{totalChanges === 1 ? '' : 's'}
                </span>
                <span style={{ fontVariantNumeric: 'tabular-nums' }}>
                  <span style={{ color: colors.success }}>+{totals.added}</span>{' '}
                  <span style={{ color: colors.error }}>−{totals.deleted}</span>
                </span>
              </>
            )}
          </div>
        </div>
        <div style={{ display: 'flex', gap: 8, flexShrink: 0 }}>
          <button
            onClick={() => void refresh()}
            disabled={loadingStatus}
            title="Refresh"
            style={{
              display: 'inline-flex',
              alignItems: 'center',
              gap: 6,
              padding: '7px 13px',
              borderRadius: 9,
              border: `1px solid ${colors.borderSubtle}`,
              background: 'var(--wks-bg-elevated)',
              color: colors.muted,
              cursor: 'pointer',
              fontSize: '0.74rem',
              fontFamily: 'inherit',
              fontWeight: 600,
            }}
          >
            <RefreshCw size={13} className={loadingStatus ? 'wks-review-spin' : undefined} />{' '}
            Refresh
          </button>
          <button
            onClick={() => void runAction((dir) => git.push(dir))}
            disabled={busy}
            title="git push"
            style={{
              display: 'inline-flex',
              alignItems: 'center',
              gap: 6,
              padding: '7px 14px',
              borderRadius: 9,
              border: 'none',
              background: colors.accent,
              color: 'var(--wks-claude-bg)',
              cursor: 'pointer',
              fontSize: '0.74rem',
              fontFamily: 'inherit',
              fontWeight: 700,
            }}
          >
            <ArrowUp size={13} /> Push
          </button>
        </div>
      </div>

      {/* Panel — file tree + diff in one rounded bordered surface (mockup). */}
      <div
        style={{
          flex: 1,
          minHeight: 0,
          display: 'flex',
          border: `1px solid ${colors.borderSubtle}`,
          borderRadius: 13,
          overflow: 'hidden',
          background: 'var(--wks-bg-surface, transparent)',
        }}
      >
        {/* Left: file tree */}
        <div
          style={{
            width: 264,
            flexShrink: 0,
            borderRight: `1px solid ${colors.borderSubtle}`,
            background: 'var(--wks-bg-base)',
            display: 'flex',
            flexDirection: 'column',
            overflow: 'hidden',
          }}
        >
          {/* Changed files header */}
          <div
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 7,
              padding: '9px 12px',
              borderBottom: `1px solid ${colors.borderSubtle}`,
              fontFamily: 'var(--wks-font-mono, monospace)',
              fontSize: '0.58rem',
              textTransform: 'uppercase',
              letterSpacing: '0.08em',
              color: colors.muted,
            }}
          >
            <span>Changed files</span>
            <span style={{ color: colors.textBright }}>{totalChanges}</span>
          </div>

          {error && (
            <div
              style={{
                margin: '0 0 0 0',
                padding: '8px 10px',
                borderBottom: `1px solid ${colors.borderSubtle}`,
                background: 'color-mix(in srgb, var(--wks-error) 10%, transparent)',
                color: colors.error,
                fontSize: '0.7rem',
                lineHeight: 1.45,
                display: 'flex',
                alignItems: 'flex-start',
                gap: 6,
                flexShrink: 0,
              }}
            >
              <span style={{ flex: 1 }}>{error}</span>
              <button
                onClick={() => setError('')}
                aria-label="Dismiss error"
                style={{
                  background: 'none',
                  border: 'none',
                  cursor: 'pointer',
                  color: colors.error,
                  padding: 0,
                  lineHeight: 1,
                  fontSize: '0.85rem',
                  flexShrink: 0,
                  opacity: 0.8,
                }}
              >
                ×
              </button>
            </div>
          )}
          <div className="wks-review-scroll" style={{ overflowY: 'auto', flex: 1 }}>
            {!error && totalChanges === 0 && !loadingStatus && (
              <Centered>
                <CheckCircle2 size={22} style={{ color: colors.success, opacity: 0.85 }} />
                <span style={{ color: colors.muted }}>Working tree clean</span>
              </Centered>
            )}

            {staged.length > 0 && (
              <>
                <SectionHeader
                  label="Staged"
                  count={staged.length}
                  action={{
                    label: 'Unstage all',
                    busy,
                    onClick: () => void runAction((dir) => git.unstage(dir)),
                  }}
                />
                <FileTree entries={staged} {...treeProps('staged')} />
              </>
            )}

            {unstaged.length > 0 && (
              <>
                <SectionHeader
                  label="Changes"
                  count={unstaged.length}
                  action={{
                    label: 'Stage all',
                    busy,
                    onClick: () => void runAction((dir) => git.stage(dir)),
                  }}
                />
                <FileTree entries={unstaged} {...treeProps('unstaged')} />
              </>
            )}

            {untracked.length > 0 && (
              <>
                <SectionHeader
                  label="Untracked"
                  count={untracked.length}
                  // "Stage all" is `git add -A` (everything), so only offer it
                  // here when there are no tracked changes — otherwise the
                  // Changes header already covers it.
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
                <FileTree entries={untracked} {...treeProps('untracked')} />
              </>
            )}
          </div>

          {/* Commit bar — enabled only when something is staged. */}
          <div
            style={{
              borderTop: `1px solid ${colors.borderSubtle}`,
              padding: 10,
              display: 'flex',
              flexDirection: 'column',
              gap: 7,
            }}
          >
            <textarea
              className="wks-review-commit-input"
              value={commitMsg}
              onChange={(e) => setCommitMsg(e.target.value)}
              onKeyDown={(e) => {
                // Cmd/Ctrl+Enter commits, matching the usual editor convention.
                if (e.key === 'Enter' && (e.metaKey || e.ctrlKey) && canCommit) {
                  e.preventDefault();
                  commit();
                }
              }}
              placeholder={
                staged.length > 0 ? 'Commit message… (⌘/Ctrl+Enter)' : 'Stage files to commit'
              }
              disabled={busy || staged.length === 0}
              rows={2}
              style={{
                resize: 'none',
                padding: '7px 9px',
                borderRadius: 7,
                border: `1px solid ${colors.borderSubtle}`,
                background: 'var(--wks-bg-input, transparent)',
                color: colors.text,
                fontSize: '0.72rem',
                fontFamily: 'inherit',
                boxSizing: 'border-box',
              }}
            />
            <button
              className="wks-review-commit-btn"
              onClick={commit}
              disabled={!canCommit}
              style={{
                display: 'inline-flex',
                alignItems: 'center',
                justifyContent: 'center',
                gap: 6,
                padding: '6px 10px',
                borderRadius: 7,
                border: 'none',
                background: canCommit
                  ? colors.accent
                  : 'color-mix(in srgb, var(--wks-text-faint) 12%, transparent)',
                color: canCommit ? 'var(--wks-claude-bg)' : colors.muted,
                cursor: canCommit ? 'pointer' : 'default',
                fontSize: '0.72rem',
                fontFamily: 'inherit',
                fontWeight: 600,
              }}
            >
              <IconCommit size={14} strokeWidth={2.2} accent="currentColor" />
              {`Commit${staged.length > 0 ? ` ${staged.length} file${staged.length > 1 ? 's' : ''}` : ''}`}
            </button>
          </div>
        </div>

        {/* Right: diff */}
        <div style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column' }}>
          {selection && (
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 10,
                padding: '7px 12px',
                borderBottom: `1px solid ${colors.borderSubtle}`,
                flexShrink: 0,
              }}
            >
              <div style={{ display: 'flex', alignItems: 'center', gap: 8, flex: 1, minWidth: 0 }}>
                <StatusChip
                  code={
                    selection.untracked
                      ? 'A'
                      : ((selection.staged ? staged : unstaged).find(
                          (e) => e.file.path === selection.path,
                        )?.code ?? 'M')
                  }
                />
                <PathBreadcrumb path={selection.path} />
              </div>
              {selectionStats && selectionStats.added != null && selectionStats.deleted != null && (
                <>
                  <span
                    style={{
                      display: 'flex',
                      gap: 6,
                      fontSize: '0.68rem',
                      fontFamily: 'var(--wks-font-mono, monospace)',
                      fontVariantNumeric: 'tabular-nums',
                      flexShrink: 0,
                    }}
                  >
                    <span style={{ color: colors.success }}>+{selectionStats.added}</span>
                    <span style={{ color: colors.error }}>−{selectionStats.deleted}</span>
                  </span>
                  <DiffStatBlocks added={selectionStats.added} deleted={selectionStats.deleted} />
                </>
              )}
              <span
                style={{
                  fontSize: '0.6rem',
                  fontWeight: 600,
                  letterSpacing: '0.04em',
                  color: colors.muted,
                  border: `1px solid ${colors.borderSubtle}`,
                  borderRadius: 9,
                  padding: '1px 8px',
                  flexShrink: 0,
                  textTransform: 'uppercase',
                }}
              >
                {selection.untracked ? 'untracked' : selection.staged ? 'staged' : 'unstaged'}
              </span>
              <button
                className="wks-icon-btn"
                title="Copy path"
                onClick={() => {
                  void navigator.clipboard.writeText(selection.path);
                  setCopied(true);
                  setTimeout(() => setCopied(false), 1200);
                }}
              >
                {copied ? (
                  <Check size={13} style={{ color: colors.success }} />
                ) : (
                  <Copy size={13} />
                )}
              </button>
            </div>
          )}
          <div style={{ flex: 1, minHeight: 0, overflow: 'hidden' }}>
            {!selection ? (
              <Centered>
                {totalChanges === 0 ? (
                  <>
                    <CheckCircle2 size={26} style={{ color: colors.success, opacity: 0.85 }} />
                    <span style={{ color: colors.muted }}>
                      Working tree clean — nothing to review.
                    </span>
                  </>
                ) : (
                  <span style={{ color: colors.muted }}>Select a file to view its diff.</span>
                )}
              </Centered>
            ) : loadingDiff ? (
              <DiffSkeleton />
            ) : diffError ? (
              <Centered>
                <FileX2 size={22} style={{ color: colors.error, opacity: 0.85 }} />
                <span style={{ color: colors.error }}>{diffError}</span>
              </Centered>
            ) : isLargeGated ? (
              <Centered>
                <span style={{ color: colors.muted }}>
                  This diff is {(diffText.length / 1_000_000).toFixed(1)} MB.
                </span>
                <button
                  onClick={() => setForcedLarge((prev) => new Set(prev).add(selKey(selection)))}
                  style={{
                    padding: '5px 14px',
                    borderRadius: 6,
                    border: `1px solid ${colors.borderSubtle}`,
                    background: 'transparent',
                    color: colors.text,
                    cursor: 'pointer',
                    fontSize: '0.74rem',
                    fontFamily: 'inherit',
                  }}
                >
                  Render anyway
                </button>
              </Centered>
            ) : !parsed || (parsed.hunks.length === 0 && !parsed.binary) ? (
              <Centered>
                <FileX2 size={22} style={{ color: colors.muted, opacity: 0.6 }} />
                <span style={{ color: colors.muted }}>
                  {parsed && diffText
                    ? 'No textual changes — file renamed or metadata-only.'
                    : 'No textual diff to show.'}
                </span>
              </Centered>
            ) : parsed.binary ? (
              <Centered>
                <FileX2 size={22} style={{ color: colors.muted, opacity: 0.85 }} />
                <span style={{ color: colors.muted }}>Binary file — no textual diff.</span>
              </Centered>
            ) : (
              <DiffViewer diff={parsed} path={selection.path} />
            )}
          </div>
        </div>
      </div>
    </div>
  );
};

export default ReviewPane;
