import React, { useState } from 'react';
import { ContextMenu, ContextMenuItem } from '../ContextMenu';
import { requestOpenInEditor } from '../../lib/editorBus';
import { requestMarkdownPreview } from '../../lib/previewBus';

/**
 * FileLink — the one clickable-file-path affordance for the chat's tool-call
 * UI (trace rows, diff/read headers, work-card file lists). Left-click opens
 * the file with its default action (markdown → preview pane, everything else →
 * editor); right-click offers the full menu: editor / markdown preview /
 * open-in-browser (html) / show in folder / copy path.
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

/** Default left-click action by extension: md → preview pane, else editor. */
export function openFileDefault(path: string, cwd?: string): void {
  const abs = resolveWithCwd(path, cwd);
  if (isMarkdownPath(abs)) requestMarkdownPreview({ path: abs, cwd });
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
          onClick={run(() => void window.electronAPI.fileOpenExternal(abs))}
        />
      )}
      <ContextMenuItem
        label="Show in folder"
        onClick={run(() => void window.electronAPI.fileShowInFolder(abs))}
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
  /** Tiny type glyph after the name for md/html files. Default true. */
  glyph?: boolean;
}> = ({ path, cwd, children, style, title, glyph = true }) => {
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const [hover, setHover] = useState(false);
  const abs = resolveWithCwd(path, cwd);
  const basename = abs.replace(/\\/g, '/').split('/').pop() ?? abs;
  const md = isMarkdownPath(abs);
  const html = isHtmlPath(abs);

  return (
    <>
      <span
        role="button"
        title={title ?? abs}
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
          textDecorationColor: 'var(--wks-text-muted, currentColor)',
          textUnderlineOffset: 2,
          ...style,
        }}
      >
        {children ?? basename}
        {glyph && (md || html) && (
          <span
            aria-hidden
            style={{ marginLeft: 4, fontSize: '0.85em', opacity: 0.55, userSelect: 'none' }}
          >
            {html ? '⊕' : 'M↓'}
          </span>
        )}
      </span>
      {menu && (
        <ContextMenu x={menu.x} y={menu.y} onClose={() => setMenu(null)}>
          <FileActionMenuItems path={path} cwd={cwd} onClose={() => setMenu(null)} />
        </ContextMenu>
      )}
    </>
  );
};
