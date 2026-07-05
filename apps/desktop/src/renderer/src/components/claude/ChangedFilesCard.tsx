/**
 * Inline "Changed files" card at the end of a completed agent turn: a header
 * with total +/− counts and Collapse all / View diff actions over a collapsible
 * directory tree (per-file line counts, file-type-tinted glyphs). Clicking a
 * file jumps to the Review pane; right-click opens it in the editor.
 *
 * Data is a frozen `TurnChangeSnapshot` (see lib/turnChanges.ts) — historical
 * cards must not drift as later turns touch the same files. When the snapshot
 * is estimate-only (non-git cwd, app restart) the header flags it with "~".
 */

import React, { useMemo, useState } from 'react';
import { claudeColors as colors } from '../claude-shared';
import FileTree, { collectDirPaths, type TreeEntry } from '../review/FileTree';
import type { NumstatEntry } from '../../lib/gitQueries';
import type { TurnChangeSnapshot } from '../../lib/turnChanges';
import { requestReviewFile } from '../../lib/reviewBus';
import { requestOpenInEditor } from '../../lib/editorBus';
import { langForPath } from '../../lib/diff/highlight';
import { IconFile } from '../wksIcons';

/** Past this many files the tree stops pulling its weight — header only. */
const MAX_TREE_FILES = 300;

/** Language-family tint for the generic file glyph — enough hue variety to
 *  scan a tree without pulling in an icon font. */
const COLOR_BY_LANG: Record<string, string> = {
  typescript: '#519aba',
  tsx: '#519aba',
  javascript: '#e8d44d',
  jsx: '#e8d44d',
  json: '#e8d44d',
  jsonc: '#e8d44d',
  css: '#9b7cd6',
  scss: '#e07a9e',
  html: '#e2764c',
  vue: '#67b587',
  svelte: '#e2764c',
  markdown: '#6a9fd8',
  python: '#649dd1',
  rust: '#c98a68',
  go: '#63c0d8',
  ruby: '#d16a6a',
  java: '#d1885f',
  shellscript: '#8fc177',
  yaml: '#c48a5a',
  toml: '#c48a5a',
  sql: '#d8b05f',
  docker: '#63c0d8',
};

const FileGlyph: React.FC<{ path: string }> = ({ path }) => {
  const lang = langForPath(path);
  const color = (lang && COLOR_BY_LANG[lang]) || colors.muted;
  return <IconFile size={13} strokeWidth={2} style={{ flexShrink: 0, color, opacity: 0.9 }} />;
};

const headerBtnStyle: React.CSSProperties = {
  padding: '2px 9px',
  borderRadius: 6,
  border: `1px solid ${colors.borderSubtle}`,
  background: 'transparent',
  color: colors.text,
  cursor: 'pointer',
  fontSize: '0.62rem',
  fontFamily: 'inherit',
  fontWeight: 600,
  flexShrink: 0,
};

export const ChangedFilesCard: React.FC<{
  snapshot: TurnChangeSnapshot;
  cwd?: string;
}> = ({ snapshot, cwd }) => {
  const entries = useMemo<TreeEntry[]>(
    () =>
      snapshot.files.map((f) => ({
        file: { path: f.relPath, staged: ' ', unstaged: f.code },
        code: f.code,
        key: f.relPath,
      })),
    [snapshot],
  );

  const stats = useMemo(() => {
    const map = new Map<string, NumstatEntry>();
    for (const f of snapshot.files) {
      map.set(f.relPath, { path: f.relPath, added: f.added, deleted: f.removed });
    }
    return map;
  }, [snapshot]);

  // Directories aggregate their files' counts. Accumulating on every path
  // prefix is safe with chain compression: a compressed node keeps its deepest
  // path, which is one of the prefixes.
  const dirCounts = useMemo(() => {
    const map = new Map<string, { added: number; removed: number }>();
    for (const f of snapshot.files) {
      const segments = f.relPath.split('/');
      for (let i = 1; i < segments.length; i++) {
        const dirPath = segments.slice(0, i).join('/');
        const cur = map.get(dirPath) ?? { added: 0, removed: 0 };
        cur.added += f.added ?? 0;
        cur.removed += f.removed ?? 0;
        map.set(dirPath, cur);
      }
    }
    return map;
  }, [snapshot]);

  const allDirs = useMemo(() => collectDirPaths(entries), [entries]);
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(new Set());
  const allCollapsed = allDirs.length > 0 && allDirs.every((d) => collapsed.has(d));
  const toggleDir = (path: string) =>
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });

  const viewDiff = () => {
    const first = snapshot.files[0];
    if (first) requestReviewFile({ path: first.path, cwd });
  };

  const count = snapshot.files.length;
  if (count === 0) return null;
  const tooMany = count > MAX_TREE_FILES;

  return (
    <div
      style={{
        margin: '4px 0 10px 0',
        borderRadius: 8,
        border: `1px solid ${colors.borderSubtle}`,
        backgroundColor: 'rgba(255,255,255,0.015)',
        overflow: 'hidden',
        animation: 'claudeFadeIn 0.2s ease-out',
      }}
    >
      {/* Header: CHANGED FILES (N) · +A/−R  |  Collapse all · View diff */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 10,
          padding: '7px 12px',
        }}
      >
        <span
          style={{
            fontSize: '0.6rem',
            fontWeight: 700,
            letterSpacing: '0.08em',
            textTransform: 'uppercase',
            color: colors.muted,
            flexShrink: 0,
          }}
        >
          Changed files ({count})
        </span>
        <span
          title={
            snapshot.gitAvailable
              ? undefined
              : 'Estimated from tool inputs — git status unavailable'
          }
          style={{
            display: 'flex',
            gap: 6,
            fontSize: '0.64rem',
            fontFamily: 'var(--claude-mono-font, monospace)',
            fontVariantNumeric: 'tabular-nums',
            flexShrink: 0,
          }}
        >
          {!snapshot.gitAvailable && <span style={{ color: colors.mutedDim }}>~</span>}
          {snapshot.totalAdded > 0 && (
            <span style={{ color: colors.success }}>+{snapshot.totalAdded}</span>
          )}
          {snapshot.totalRemoved > 0 && (
            <span style={{ color: colors.error }}>−{snapshot.totalRemoved}</span>
          )}
        </span>
        <div style={{ flex: 1 }} />
        {!tooMany && allDirs.length > 0 && (
          <button
            style={headerBtnStyle}
            onClick={() => setCollapsed(allCollapsed ? new Set() : new Set(allDirs))}
          >
            {allCollapsed ? 'Expand all' : 'Collapse all'}
          </button>
        )}
        <button style={headerBtnStyle} onClick={viewDiff}>
          View diff
        </button>
      </div>

      <div style={{ borderTop: `1px solid ${colors.borderSubtle}`, padding: '4px 4px 4px 0' }}>
        {tooMany ? (
          <div style={{ padding: '6px 12px', fontSize: '0.72rem', color: colors.muted }}>
            {count} files changed — use View diff to review them.
          </div>
        ) : (
          <FileTree
            entries={entries}
            selectedKey={null}
            stats={stats}
            dirCounts={dirCounts}
            collapsed={collapsed}
            onToggleDir={toggleDir}
            renderIcon={(entry) => <FileGlyph path={entry.file.path} />}
            onSelect={(entry) => {
              const f = snapshot.files.find((x) => x.relPath === entry.file.path);
              requestReviewFile({ path: f?.path ?? entry.file.path, cwd });
            }}
            onOpenInEditor={(entry) => {
              const f = snapshot.files.find((x) => x.relPath === entry.file.path);
              requestOpenInEditor({ path: f?.path ?? entry.file.path, cwd });
            }}
          />
        )}
      </div>
    </div>
  );
};

export default ChangedFilesCard;
