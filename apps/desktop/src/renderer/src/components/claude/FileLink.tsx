import React, { useState } from 'react';
import { ContextMenu, ContextMenuItem } from '../ContextMenu';
import { requestOpenInEditor } from '../../lib/editorBus';
import { requestMarkdownPreview } from '../../lib/previewBus';
import { requestOpenInBrowser, fileUrlFromPath } from '../../lib/browserBus';
import { PaneIcon } from '../icons';
import type { PaneType } from '../../types/pane';
import { postNotification } from '../../lib/notificationBus';

/**
 * FileLink — the one clickable-file-path affordance for the chat's tool-call
 * UI (trace rows, diff/read headers, work-card file lists) and for file paths
 * detected in assistant prose. Left-click opens the file with its default
 * action (markdown → preview pane, everything else → editor); right-click
 * offers the full menu: editor / markdown preview / open-in-browser (html) /
 * show in folder / copy path.
 *
 * The link wears a leading icon of whatever the click will open — the same
 * glyph that pane's tab carries — which doubles as the "this path is
 * clickable" cue in flowing prose, where nothing else marks it until hover.
 *
 * Paths from tool inputs are usually absolute, but relative ones resolve
 * against `cwd` before hitting the IPC-backed actions.
 */

/** Absolute on POSIX (/…), Windows drive (C:\…), or UNC (\\…). */
export function isAbsolutePath(p: string): boolean {
  return p.startsWith('/') || p.startsWith('\\\\') || /^[a-zA-Z]:[\\/]/.test(p);
}

/** Resolve `path` against `cwd` when relative; already-absolute paths pass through. */
export function resolveWithCwd(path: string, cwd?: string): string {
  if (isAbsolutePath(path) || !cwd) return path;
  return `${cwd.replace(/[\\/]+$/, '')}/${path}`;
}

const extOf = (p: string): string => /\.([a-z0-9]+)$/i.exec(p)?.[1]?.toLowerCase() ?? '';
export const isMarkdownPath = (p: string): boolean => ['md', 'markdown'].includes(extOf(p));
export const isHtmlPath = (p: string): boolean => ['html', 'htm'].includes(extOf(p));

/** Where a left-click hands the file. */
export type FileOpenTarget = 'editor' | 'preview';

/**
 * The ONE place the default action is decided. Both the dispatch below and the
 * little icon the link wears read it, so the affordance can never advertise a
 * surface the click doesn't actually open.
 */
export function defaultOpenTarget(path: string): FileOpenTarget {
  return isMarkdownPath(path) ? 'preview' : 'editor';
}

/** Pane type whose icon stands for each target, plus the tooltip wording. The
 *  icon is the *pane's own* icon, so the glyph beside a path is the same glyph
 *  that ends up on the tab it opens. */
const OPEN_TARGET_UI: Record<FileOpenTarget, { pane: PaneType; hint: string }> = {
  editor: { pane: 'editor', hint: 'opens in the editor' },
  preview: { pane: 'mdpreview', hint: 'opens a markdown preview' },
};

/** Default left-click action by extension: md → preview pane, else editor. */
export function openFileDefault(path: string, cwd?: string): void {
  const abs = resolveWithCwd(path, cwd);
  if (defaultOpenTarget(abs) === 'preview') requestMarkdownPreview({ path: abs, cwd });
  else requestOpenInEditor({ path: abs, cwd });
}

/**
 * The shared right-click menu body — also used by surfaces that own their own
 * ContextMenu (e.g. ChangedFilesCard's file tree). Render inside a
 * <ContextMenu>; every action closes the menu via `onClose`.
 */
export const FileActionMenuItems: React.FC<{
  path: string;
  cwd?: string;
  onClose: () => void;
}> = ({ path, cwd, onClose }) => {
  const abs = resolveWithCwd(path, cwd);
  const run = (action: () => void) => () => {
    action();
    onClose();
  };
  return (
    <>
      <ContextMenuItem
        label="Open in editor"
        onClick={run(() => requestOpenInEditor({ path: abs, cwd }))}
      />
      {isMarkdownPath(abs) && (
        <ContextMenuItem
          label="Preview markdown"
          onClick={run(() => requestMarkdownPreview({ path: abs, cwd }))}
        />
      )}
      {isHtmlPath(abs) && (
        <ContextMenuItem
          label="Open in browser"
          onClick={run(() =>
            requestOpenInBrowser({
              url: fileUrlFromPath(abs),
              title: abs.replace(/\\/g, '/').split('/').pop() || 'Browser',
            }),
          )}
        />
      )}
      <ContextMenuItem
        label="Show in folder"
        // The handler deliberately reports {ok:false, error} for a path that is
        // gone — discarding it made a stale link a dead click with no feedback,
        // which is the single most common case (the agent moved or deleted it).
        onClick={run(() => {
          void window.electronAPI.fileShowInFolder(abs).then((res) => {
            if (res && res.ok === false) {
              postNotification({
                title: 'Could not show the file',
                body: res.error || abs,
                level: 'warn',
                source: 'files',
              });
            }
          });
        })}
      />
      <ContextMenuItem
        label="Copy path"
        onClick={run(() => void navigator.clipboard.writeText(abs))}
      />
    </>
  );
};

export const FileLink: React.FC<{
  path: string;
  cwd?: string;
  /** Display content — defaults to the file's basename. */
  children?: React.ReactNode;
  /** Extra styles merged over the link's own (mono font, hover underline). */
  style?: React.CSSProperties;
  title?: string;
  /** Leading icon for whatever a click will open. Default true. */
  icon?: boolean;
}> = ({ path, cwd, children, style, title, icon = true }) => {
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const [hover, setHover] = useState(false);
  const abs = resolveWithCwd(path, cwd);
  const basename = abs.replace(/\\/g, '/').split('/').pop() ?? abs;
  const target = defaultOpenTarget(abs);
  const targetUi = OPEN_TARGET_UI[target];

  return (
    <>
      <span
        role="button"
        data-open-target={target}
        // The tooltip carries the destination too — the icon says which surface,
        // the words say it out loud.
        title={title ?? `${abs} — ${targetUi.hint}`}
        onClick={(e) => {
          // Rows/cards behind the link often toggle on click — the link wins.
          e.stopPropagation();
          openFileDefault(path, cwd);
        }}
        onContextMenu={(e) => {
          e.preventDefault();
          e.stopPropagation();
          setMenu({ x: e.clientX, y: e.clientY });
        }}
        onMouseEnter={() => setHover(true)}
        onMouseLeave={() => setHover(false)}
        style={{
          cursor: 'pointer',
          fontFamily: 'var(--claude-mono-font, monospace)',
          textDecoration: hover ? 'underline' : 'none',
          textDecorationColor: 'var(--wks-text-muted)',
          textUnderlineOffset: 2,
          // One inline unit, so a line break in prose can't strand the icon at
          // the end of a line with its path on the next one. A path that can't
          // fit moves down whole; one longer than the column still breaks
          // (overflowWrap) rather than escaping it.
          display: 'inline-block',
          maxWidth: '100%',
          overflowWrap: 'anywhere',
          ...style,
        }}
      >
        {/* Leading, not trailing: several hosts ellipsize the path (trace rows,
            narrow diff headers), and a tail icon would be the first thing
            clipped — exactly when you most want to know what a click does. */}
        {icon && (
          <PaneIcon
            type={targetUi.pane}
            size={11}
            strokeWidth={2}
            style={{
              marginRight: 3,
              verticalAlign: '-0.12em',
              flex: 'none',
              opacity: hover ? 1 : 0.5,
              transition: 'opacity 0.12s',
            }}
          />
        )}
        {children ?? basename}
      </span>
      {menu && (
        <ContextMenu x={menu.x} y={menu.y} onClose={() => setMenu(null)}>
          <FileActionMenuItems path={path} cwd={cwd} onClose={() => setMenu(null)} />
        </ContextMenu>
      )}
    </>
  );
};
