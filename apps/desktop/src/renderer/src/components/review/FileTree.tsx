/**
 * GitHub-style file tree for the review pane sidebar: folders collapse,
 * single-child folder chains compress ("src/renderer/src"), and each file row
 * carries a tinted status chip, +/− line counts and a hover stage/unstage
 * action.
 */

import React, { useMemo, useState } from 'react';
import { ChevronRight, Folder, FolderOpen } from 'lucide-react';
import { claudeColors as colors } from '../claude-shared';
import type { FileStatus, NumstatEntry } from '../../lib/gitQueries';
import { ensureReviewStyles } from './reviewStyles';
import { ContextMenu, ContextMenuItem } from '../ContextMenu';

/** One selectable entry in the tree (a changed file in one section). */
export interface TreeEntry {
  file: FileStatus;
  /** Porcelain code to badge (staged or unstaged side, per section). */
  code: string;
  /** Stable selection key, e.g. "s:src/App.tsx". */
  key: string;
}

interface DirNode {
  /** Display name — may span compressed segments ("src/components"). */
  name: string;
  path: string;
  dirs: DirNode[];
  files: TreeEntry[];
}

function buildTree(entries: TreeEntry[]): DirNode {
  const root: DirNode = { name: '', path: '', dirs: [], files: [] };
  for (const entry of entries) {
    const segments = entry.file.path.split('/');
    let node = root;
    for (let i = 0; i < segments.length - 1; i++) {
      const path = segments.slice(0, i + 1).join('/');
      let child = node.dirs.find((d) => d.path === path);
      if (!child) {
        child = { name: segments[i], path, dirs: [], files: [] };
        node.dirs.push(child);
      }
      node = child;
    }
    node.files.push(entry);
  }
  compress(root);
  sortTree(root);
  return root;
}

/** Merge single-child directory chains into one node, GitHub-style. */
function compress(node: DirNode): void {
  for (let i = 0; i < node.dirs.length; i++) {
    let dir = node.dirs[i];
    while (dir.dirs.length === 1 && dir.files.length === 0) {
      const only = dir.dirs[0];
      dir = { ...only, name: `${dir.name}/${only.name}` };
      node.dirs[i] = dir;
    }
    compress(dir);
  }
}

function sortTree(node: DirNode): void {
  node.dirs.sort((a, b) => a.name.localeCompare(b.name));
  node.files.sort((a, b) => a.file.path.localeCompare(b.file.path));
  node.dirs.forEach(sortTree);
}

export function codeColor(code: string): string {
  switch (code) {
    case 'M':
      return colors.warning;
    case 'A':
      return colors.success;
    case 'D':
    case 'U':
      return colors.error;
    case 'R':
    case 'C':
      return colors.accent;
    default:
      return colors.muted;
  }
}

/** Tinted rounded-square status chip ("M", "A", "D"…). */
export const StatusChip: React.FC<{ code: string }> = ({ code }) => {
  const color = codeColor(code);
  return (
    <span
      style={{
        width: 16,
        height: 16,
        borderRadius: 4,
        display: 'inline-flex',
        alignItems: 'center',
        justifyContent: 'center',
        flexShrink: 0,
        fontSize: '0.58rem',
        fontWeight: 700,
        fontFamily: 'var(--wks-font-mono, monospace)',
        color,
        background: `color-mix(in srgb, ${color} 14%, transparent)`,
      }}
    >
      {code}
    </span>
  );
};

const Counts: React.FC<{ stat?: NumstatEntry }> = ({ stat }) => {
  if (!stat) return null;
  if (stat.added == null || stat.deleted == null) {
    return <span style={{ color: colors.muted, fontSize: '0.6rem', flexShrink: 0 }}>BIN</span>;
  }
  return (
    <span
      style={{
        display: 'flex',
        gap: 5,
        fontSize: '0.62rem',
        fontFamily: 'var(--wks-font-mono, monospace)',
        fontVariantNumeric: 'tabular-nums',
        flexShrink: 0,
      }}
    >
      {stat.added > 0 && <span style={{ color: colors.success }}>+{stat.added}</span>}
      {stat.deleted > 0 && <span style={{ color: colors.error }}>−{stat.deleted}</span>}
    </span>
  );
};

export interface FileTreeProps {
  entries: TreeEntry[];
  selectedKey: string | null;
  onSelect: (entry: TreeEntry) => void;
  /** Inline hover action per file ("Stage" / "Unstage"). */
  actionLabel: string;
  onAction: (entry: TreeEntry) => void;
  busy: boolean;
  /** Line counts per path, from numstat. */
  stats: ReadonlyMap<string, NumstatEntry>;
  /** Right-click → "Open in editor". Omit to disable the context menu. */
  onOpenInEditor?: (entry: TreeEntry) => void;
}

const INDENT = 13;

const FileTree: React.FC<FileTreeProps> = ({
  entries,
  selectedKey,
  onSelect,
  actionLabel,
  onAction,
  busy,
  stats,
  onOpenInEditor,
}) => {
  ensureReviewStyles();
  const tree = useMemo(() => buildTree(entries), [entries]);
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(new Set());
  const [menu, setMenu] = useState<{ x: number; y: number; entry: TreeEntry } | null>(null);

  const toggle = (path: string) => {
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  };

  const renderDir = (dir: DirNode, depth: number): React.ReactNode => {
    const isCollapsed = collapsed.has(dir.path);
    const FolderIcon = isCollapsed ? Folder : FolderOpen;
    return (
      <React.Fragment key={dir.path}>
        <div
          className="wks-review-dir"
          onClick={() => toggle(dir.path)}
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 5,
            height: 24,
            padding: `0 8px 0 ${8 + depth * INDENT}px`,
            cursor: 'pointer',
            color: colors.muted,
            fontSize: '0.72rem',
            userSelect: 'none',
            borderRadius: 6,
            margin: '0 4px',
          }}
        >
          <ChevronRight
            className="wks-review-chevron"
            size={11}
            style={{ flexShrink: 0, transform: isCollapsed ? 'none' : 'rotate(90deg)' }}
          />
          <FolderIcon size={12} style={{ flexShrink: 0, opacity: 0.8 }} />
          <span
            style={{
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
              fontWeight: 600,
            }}
          >
            {dir.name}
          </span>
        </div>
        {!isCollapsed && renderChildren(dir, depth + 1)}
      </React.Fragment>
    );
  };

  const renderChildren = (node: DirNode, depth: number): React.ReactNode => (
    <>
      {node.dirs.map((d) => renderDir(d, depth))}
      {node.files.map((entry) => {
        const name = entry.file.path.split('/').pop() ?? entry.file.path;
        const active = entry.key === selectedKey;
        const title = entry.file.orig_path
          ? `${entry.file.orig_path} → ${entry.file.path}`
          : entry.file.path;
        return (
          <div
            key={entry.key}
            className="wks-review-row"
            onClick={() => onSelect(entry)}
            onContextMenu={onOpenInEditor ? (e) => {
              e.preventDefault();
              e.stopPropagation();
              setMenu({ x: e.clientX, y: e.clientY, entry });
            } : undefined}
            title={title}
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 7,
              height: 26,
              padding: `0 8px 0 ${8 + depth * INDENT + 14}px`,
              cursor: 'pointer',
              fontSize: '0.74rem',
              background: active
                ? 'color-mix(in srgb, var(--wks-accent-text) 12%, transparent)'
                : 'transparent',
              color: active ? colors.textBright : colors.text,
              borderRadius: 6,
              margin: '0 4px',
              boxSizing: 'border-box',
            }}
          >
            <StatusChip code={entry.code} />
            <span
              style={{
                flex: 1,
                minWidth: 0,
                overflow: 'hidden',
                textOverflow: 'ellipsis',
                whiteSpace: 'nowrap',
                fontWeight: active ? 600 : 400,
              }}
            >
              {name}
            </span>
            <Counts stat={stats.get(entry.file.path)} />
            <button
              className="wks-review-action"
              onClick={(e) => {
                e.stopPropagation();
                onAction(entry);
              }}
              disabled={busy}
              title={actionLabel}
              style={{
                flexShrink: 0,
                padding: '0 7px',
                borderRadius: 5,
                border: `1px solid ${colors.borderSubtle}`,
                background: colors.bg,
                color: colors.text,
                cursor: busy ? 'default' : 'pointer',
                fontSize: '0.6rem',
                fontFamily: 'inherit',
                fontWeight: 600,
                lineHeight: '16px',
                height: 18,
              }}
            >
              {actionLabel}
            </button>
          </div>
        );
      })}
    </>
  );

  return (
    <div style={{ paddingBottom: 2 }}>
      {renderChildren(tree, 0)}
      {menu && onOpenInEditor && (
        <ContextMenu x={menu.x} y={menu.y} onClose={() => setMenu(null)}>
          <ContextMenuItem label="Open in editor" onClick={() => { onOpenInEditor(menu.entry); setMenu(null); }} />
        </ContextMenu>
      )}
    </div>
  );
};

export default FileTree;
