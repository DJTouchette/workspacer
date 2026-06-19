/**
 * EditorPane — an in-app code editor (CodeMirror 6) for a single file.
 *
 * Files are read/written through the app's main-process file backend
 * (window.electronAPI.readFile / writeFile), which works on desktop (IPC) and
 * remote/web (hub fs.read/fs.write capability). The editor is the 'codemirror'
 * engine; the 'terminal' engine ($EDITOR in a PTY) is handled in ScrollContainer.
 *
 * No-remount: like the terminal/browser panes, the CodeMirror view is created
 * once on mount and kept alive across view-mode changes (ScrollContainer holds
 * stable keys). Only geometry/visibility changes around it.
 */
import React, { useEffect, useRef, useState } from 'react';
import { EditorView, basicSetup } from 'codemirror';
import { EditorState, Compartment } from '@codemirror/state';
import { keymap } from '@codemirror/view';
import { indentWithTab } from '@codemirror/commands';
import { gotoLine } from '@codemirror/search';
import { languages } from '@codemirror/language-data';
import { syntaxHighlighting, HighlightStyle } from '@codemirror/language';
import { tags as t } from '@lezer/highlight';
import { vim } from '@replit/codemirror-vim';
import { useConfig } from '../hooks/useConfig';
import { useTheme } from '../hooks/useTheme';
import { isLightTheme, type Theme } from '../themes';
import SearchPanel from './editor/SearchPanel';

/** A syntax palette built from the active theme's ANSI terminal colors, so the
 *  editor's highlighting matches the integrated terminal and every app theme
 *  exactly — not a generic dark/light preset. */
function buildHighlightStyle(theme: Theme): HighlightStyle {
  const c = theme.terminal;
  return HighlightStyle.define([
    { tag: [t.keyword, t.moduleKeyword, t.controlKeyword, t.operatorKeyword, t.definitionKeyword], color: c.magenta },
    { tag: [t.string, t.special(t.string), t.docString], color: c.green },
    { tag: [t.regexp, t.escape, t.character], color: c.cyan },
    { tag: [t.number, t.bool, t.null, t.atom], color: c.yellow },
    { tag: [t.comment, t.lineComment, t.blockComment, t.meta], color: c.brightBlack, fontStyle: 'italic' },
    { tag: [t.function(t.variableName), t.function(t.propertyName), t.labelName], color: c.blue },
    { tag: [t.typeName, t.className, t.namespace], color: c.cyan },
    { tag: [t.propertyName, t.attributeName], color: c.brightCyan },
    { tag: [t.constant(t.variableName), t.standard(t.name)], color: c.brightYellow },
    { tag: [t.variableName, t.attributeValue], color: c.foreground },
    { tag: [t.operator, t.punctuation, t.separator, t.bracket, t.derefOperator], color: c.brightBlack },
    { tag: [t.tagName, t.angleBracket], color: c.red },
    { tag: [t.heading], color: c.blue, fontWeight: 'bold' },
    { tag: [t.strong], fontWeight: 'bold' },
    { tag: [t.emphasis], fontStyle: 'italic' },
    { tag: [t.link, t.url], color: c.cyan, textDecoration: 'underline' },
    { tag: [t.invalid], color: c.brightRed },
  ]);
}

/** Editor chrome (background, text, gutter, selection, cursor) driven by the
 *  app's --wks-* tokens — so it updates live with the theme — plus the
 *  theme-matched ANSI syntax palette above. */
function themeExtensions(theme: Theme) {
  const chrome = EditorView.theme(
    {
      '&': { height: '100%', color: 'var(--wks-text-primary)', backgroundColor: 'var(--wks-bg-base)' },
      '.cm-content': { caretColor: 'var(--wks-accent)' },
      '.cm-cursor, .cm-dropCursor': { borderLeftColor: 'var(--wks-accent)' },
      '&.cm-focused .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection': {
        backgroundColor: 'var(--wks-bg-selected)',
      },
      '.cm-gutters': { backgroundColor: 'var(--wks-bg-base)', color: 'var(--wks-text-disabled)', border: 'none' },
      '.cm-activeLine': { backgroundColor: 'var(--wks-bg-hover)' },
      '.cm-activeLineGutter': { backgroundColor: 'var(--wks-bg-hover)', color: 'var(--wks-text-muted)' },
      '.cm-foldGutter, .cm-lineNumbers': { color: 'var(--wks-text-disabled)' },
      '.cm-selectionMatch': { backgroundColor: 'var(--wks-accent-bg)' },
      '.cm-matchingBracket, &.cm-focused .cm-matchingBracket': { backgroundColor: 'var(--wks-accent-bg)', outline: '1px solid var(--wks-accent)' },
      '.cm-tooltip': { backgroundColor: 'var(--wks-bg-elevated)', border: '1px solid var(--wks-border-subtle)', color: 'var(--wks-text-primary)' },
      '.cm-panels': { backgroundColor: 'var(--wks-bg-raised)', color: 'var(--wks-text-primary)' },
      '.cm-scroller': { overflow: 'auto' },
    },
    { dark: !isLightTheme(theme) },
  );
  return [chrome, syntaxHighlighting(buildHighlightStyle(theme))];
}

interface EditorPaneProps {
  paneId: string;
  title: string;
  isActive: boolean;
  filePath?: string;
  cwd?: string;
}

type Status = 'idle' | 'loading' | 'ready' | 'error';

interface DirEntry { name: string; path: string; isDir: boolean; }

function basename(p: string): string {
  return p.split(/[\\/]/).pop() || p;
}

/** Move the cursor to (1-based) line `n` and scroll it into view, clamping to
 *  the document's line count. Best-effort: no-op if the view/line is invalid. */
function scrollToLine(view: EditorView, n: number) {
  const total = view.state.doc.lines;
  const line = view.state.doc.line(Math.min(Math.max(1, n), total));
  view.dispatch({
    selection: { anchor: line.from },
    effects: EditorView.scrollIntoView(line.from, { y: 'center' }),
  });
  view.focus();
}

/** One directory level in the tree — lazily lists its children when expanded. */
const TreeDir: React.FC<{
  path: string;
  name: string;
  depth: number;
  activePath?: string;
  defaultOpen?: boolean;
  onOpen: (p: string) => void;
}> = ({ path, name, depth, activePath, defaultOpen, onOpen }) => {
  const [open, setOpen] = useState(!!defaultOpen);
  const [entries, setEntries] = useState<DirEntry[] | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (!open || entries !== null || loading) return;
    setLoading(true);
    window.electronAPI
      .readDir(path)
      .then((r) => setEntries(r.entries))
      .catch(() => setEntries([]))
      .finally(() => setLoading(false));
  }, [open, path, entries, loading]);

  const indent = 6 + depth * 12;
  return (
    <>
      <div
        onClick={() => setOpen((o) => !o)}
        title={name}
        style={{
          display: 'flex', alignItems: 'center', gap: 4, cursor: 'pointer',
          padding: '1px 6px', paddingLeft: indent, whiteSpace: 'nowrap', overflow: 'hidden',
          textOverflow: 'ellipsis', color: 'var(--wks-text-secondary)',
        }}
        onMouseEnter={(e) => (e.currentTarget.style.background = 'var(--wks-bg-hover)')}
        onMouseLeave={(e) => (e.currentTarget.style.background = 'transparent')}
      >
        <span style={{ width: 10, display: 'inline-block', color: 'var(--wks-text-disabled)' }}>{open ? '▾' : '▸'}</span>
        <span style={{ overflow: 'hidden', textOverflow: 'ellipsis' }}>{name}</span>
      </div>
      {open && entries === null && (
        <div style={{ padding: '1px 6px', paddingLeft: indent + 14, color: 'var(--wks-text-disabled)', fontStyle: 'italic' }}>
          Loading…
        </div>
      )}
      {open && entries?.length === 0 && (
        <div style={{ padding: '1px 6px', paddingLeft: indent + 14, color: 'var(--wks-text-disabled)', fontStyle: 'italic' }}>
          empty
        </div>
      )}
      {open && entries?.map((e) =>
        e.isDir ? (
          <TreeDir key={e.path} path={e.path} name={e.name} depth={depth + 1} activePath={activePath} onOpen={onOpen} />
        ) : (
          <div
            key={e.path}
            onClick={() => onOpen(e.path)}
            title={e.name}
            style={{
              padding: '1px 6px', paddingLeft: 6 + (depth + 1) * 12 + 14, cursor: 'pointer',
              whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
              color: e.path === activePath ? 'var(--wks-text-primary)' : 'var(--wks-text-muted)',
              background: e.path === activePath ? 'var(--wks-bg-selected)' : 'transparent',
            }}
            onMouseEnter={(ev) => { if (e.path !== activePath) ev.currentTarget.style.background = 'var(--wks-bg-hover)'; }}
            onMouseLeave={(ev) => { if (e.path !== activePath) ev.currentTarget.style.background = 'transparent'; }}
          >
            {e.name}
          </div>
        ),
      )}
    </>
  );
};

/** File-tree view rooted at the agent's cwd. Refresh re-keys the root. */
const FileTree: React.FC<{ root: string; activePath?: string; onOpen: (p: string) => void }> = ({ root, activePath, onOpen }) => {
  const [reloadKey, setReloadKey] = useState(0);
  return (
    <div style={{ flex: 1, minHeight: 0, overflow: 'auto', paddingBottom: 8 }}>
      <div style={{
        display: 'flex', alignItems: 'center', padding: '2px 8px', position: 'sticky', top: 0,
        background: 'var(--wks-bg-raised)',
        color: 'var(--wks-text-disabled)', fontSize: '0.55rem',
      }}>
        <div style={{ flex: 1 }} />
        <span onClick={() => setReloadKey((k) => k + 1)} title="Refresh" style={{ cursor: 'pointer' }}>⟳</span>
      </div>
      <TreeDir key={reloadKey} path={root} name={basename(root)} depth={0} activePath={activePath} defaultOpen onOpen={onOpen} />
    </div>
  );
};

const EditorPane: React.FC<EditorPaneProps> = ({ filePath, cwd }) => {
  const { config } = useConfig();
  const { theme } = useTheme();
  const vimMode = config.editor?.vim ?? false;

  const hostRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const langCompartment = useRef(new Compartment());
  const themeCompartment = useRef(new Compartment());
  // Latest persisted contents, to compute the dirty flag and to skip no-op saves.
  const savedRef = useRef<string>('');

  const [status, setStatus] = useState<Status>('idle');
  const [error, setError] = useState<string>('');
  const [dirty, setDirty] = useState(false);
  const [saving, setSaving] = useState(false);
  // Sidebar mode: the file tree or project-wide search (only when cwd present).
  const [sidebarMode, setSidebarMode] = useState<'files' | 'search'>('files');
  // External-change handling. The banner shows when the open file changed on
  // disk while our buffer is dirty (don't clobber); the hint is a transient
  // "reloaded from disk" note for the clean-buffer auto-reload path.
  const [extChanged, setExtChanged] = useState(false);
  const [reloadHint, setReloadHint] = useState(false);
  // Keep `dirty` and `saving` readable inside the (stable) watch callback.
  const dirtyRef = useRef(false);
  dirtyRef.current = dirty;
  const savingRef = useRef(false);
  savingRef.current = saving;
  // Short window after our own writeFile during which watch events are ignored
  // (the OS often reports our own save). Epoch ms; 0 = no active window.
  const ignoreUntilRef = useRef(0);
  // Pending line to jump to once the (asynchronously recreated) view exists,
  // used when a search result switches the open file.
  const pendingGotoRef = useRef<number | null>(null);
  // The file currently shown in the editor. Starts at the pane's filePath (if
  // any) and changes as the user clicks files in the tree — no pane remount.
  const [openFile, setOpenFile] = useState<string | undefined>(filePath);
  useEffect(() => {
    if (filePath && filePath !== openFile) setOpenFile(filePath);
    // Only react to an externally-driven file change (e.g. open-in-editor).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [filePath]);

  const doSave = useRef<() => void>(() => {});
  doSave.current = () => {
    const view = viewRef.current;
    if (!view || !openFile || saving) return;
    const contents = view.state.doc.toString();
    if (contents === savedRef.current) return; // nothing changed
    setSaving(true);
    window.electronAPI
      .writeFile(openFile, contents)
      .then(() => {
        savedRef.current = contents;
        setDirty(false);
        // Ignore the watcher echo of our own write for a brief window.
        ignoreUntilRef.current = Date.now() + 500;
      })
      .catch((err: unknown) => setError(String((err as Error)?.message ?? err)))
      .finally(() => setSaving(false));
  };

  // Create the editor once, load the file, lazily swap in the right language.
  useEffect(() => {
    if (!hostRef.current || !openFile) return;
    let disposed = false;

    setStatus('loading');
    setError('');
    // Reset external-change UI for the newly opened file.
    setExtChanged(false);
    setReloadHint(false);
    window.electronAPI
      .readFile(openFile)
      .then((res: { contents: string }) => {
        if (disposed || !hostRef.current) return;
        savedRef.current = res.contents;

        const saveKeys = keymap.of([
          { key: 'Mod-s', preventDefault: true, run: () => { doSave.current(); return true; } },
          { key: 'Mod-g', preventDefault: true, run: gotoLine },
          indentWithTab,
        ]);
        const extensions = [
          ...(vimMode ? [vim()] : []), // vim must come first
          basicSetup,
          langCompartment.current.of([]),
          themeCompartment.current.of(themeExtensions(theme)),
          saveKeys,
          // Stop Ctrl-S / Ctrl-G bubbling to any of the app's global bindings.
          EditorView.domEventHandlers({
            keydown: (e) => {
              if ((e.ctrlKey || e.metaKey) && /^[sg]$/i.test(e.key)) {
                e.stopPropagation();
              }
              return false;
            },
          }),
          EditorView.updateListener.of((u) => {
            if (u.docChanged) {
              setDirty(u.state.doc.toString() !== savedRef.current);
            }
          }),
        ];

        viewRef.current = new EditorView({
          state: EditorState.create({ doc: res.contents, extensions }),
          parent: hostRef.current,
        });
        setStatus('ready');

        // A search result switched the open file — jump to its line now that
        // the view exists.
        if (pendingGotoRef.current != null) {
          scrollToLine(viewRef.current, pendingGotoRef.current);
          pendingGotoRef.current = null;
        }

        // Resolve the language for this extension and reconfigure when ready.
        const desc = languages.find((l) => l.extensions.includes(basename(openFile).split('.').pop() || ''));
        if (desc) {
          desc.load().then((support) => {
            if (!disposed && viewRef.current) {
              viewRef.current.dispatch({ effects: langCompartment.current.reconfigure(support) });
            }
          }).catch(() => { /* unknown language — plain text is fine */ });
        }
      })
      .catch((err: unknown) => {
        if (disposed) return;
        setError(String((err as Error)?.message ?? err));
        setStatus('error');
      });

    return () => {
      disposed = true;
      viewRef.current?.destroy();
      viewRef.current = null;
    };
    // Re-create only when the open file changes (engine/vim changes remount the pane).
  }, [openFile, vimMode]);

  // Re-theme on app theme change without rebuilding the editor. The chrome uses
  // --wks-* vars (updates for free); this swaps in the new theme's ANSI-matched
  // syntax palette.
  useEffect(() => {
    viewRef.current?.dispatch({
      effects: themeCompartment.current.reconfigure(themeExtensions(theme)),
    });
  }, [theme]);

  // Replace the whole document with `contents`, preserving the cursor offset
  // best-effort, and mark it as the persisted baseline. Used by both the silent
  // (clean-buffer) reload and the explicit "Reload (discard)" button.
  const applyDiskContents = (contents: string) => {
    const view = viewRef.current;
    if (!view) return;
    const prevPos = Math.min(view.state.selection.main.head, contents.length);
    view.dispatch({
      changes: { from: 0, to: view.state.doc.length, insert: contents },
      selection: { anchor: prevPos },
    });
    savedRef.current = contents;
    setDirty(false);
  };

  // Watch the open file for external (on-disk) changes and reconcile them with
  // the in-memory buffer. Re-armed whenever the open file changes.
  useEffect(() => {
    if (!openFile) return;
    const target = openFile;
    let cancelled = false;

    const unsub = window.electronAPI.watchFile(target, (info) => {
      if (cancelled || info.path !== target) return;
      // Ignore events while a save is in flight or within our own-save echo window.
      if (savingRef.current || Date.now() < ignoreUntilRef.current) return;

      window.electronAPI
        .readFile(target)
        .then((res: { contents: string }) => {
          // Re-check the open file / save state after the async read.
          if (cancelled || savingRef.current || Date.now() < ignoreUntilRef.current) return;
          const disk = res.contents;
          if (disk === savedRef.current) return; // our own save echoing back

          if (!dirtyRef.current) {
            // Clean buffer: silently reload and flash a subtle hint.
            applyDiskContents(disk);
            setReloadHint(true);
            window.setTimeout(() => { if (!cancelled) setReloadHint(false); }, 2500);
          } else {
            // Dirty buffer: don't clobber — let the user decide via the banner.
            setExtChanged(true);
          }
        })
        .catch(() => { /* file may have been removed mid-edit; ignore */ });
    });

    return () => { cancelled = true; unsub(); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [openFile]);

  // Banner action: discard my edits and load the on-disk version.
  const handleReloadFromDisk = () => {
    if (!openFile) return;
    window.electronAPI
      .readFile(openFile)
      .then((res: { contents: string }) => {
        applyDiskContents(res.contents);
        setExtChanged(false);
      })
      .catch((err: unknown) => setError(String((err as Error)?.message ?? err)));
  };

  // Banner action: keep my edits. Adopt the disk contents as the baseline so the
  // next Ctrl+S is a recognized overwrite and future external changes re-detect,
  // but leave the buffer dirty (it differs from disk).
  const handleKeepMine = () => {
    if (!openFile) { setExtChanged(false); return; }
    window.electronAPI
      .readFile(openFile)
      .then((res: { contents: string }) => {
        savedRef.current = res.contents;
        const view = viewRef.current;
        if (view) setDirty(view.state.doc.toString() !== res.contents);
        setExtChanged(false);
      })
      .catch(() => setExtChanged(false));
  };

  const name = openFile ? basename(openFile) : 'No file';

  // Switch the open file from the tree, saving any pending edits to the old one first.
  const handleTreeOpen = (p: string) => {
    if (p === openFile) return;
    doSave.current();
    setOpenFile(p);
  };

  // Jump to a search match: if the file is already open, scroll now; otherwise
  // switch to it (saving pending edits) and stash the target line for the open
  // effect to apply once the view is recreated.
  const handleSearchOpen = (file: string, line: number) => {
    if (file === openFile) {
      if (viewRef.current) scrollToLine(viewRef.current, line);
      return;
    }
    doSave.current();
    pendingGotoRef.current = line;
    setOpenFile(file);
  };

  return (
    <div style={{ display: 'flex', height: '100%', background: 'var(--wks-bg-base)' }}>
      {cwd && (
        <div style={{
          width: 220, flex: '0 0 auto', display: 'flex', flexDirection: 'column', minHeight: 0,
          borderRight: '1px solid var(--wks-border-subtle)', background: 'var(--wks-bg-raised)',
          fontFamily: 'var(--wks-mono, ui-monospace, monospace)', fontSize: '0.7rem',
        }}>
          {/* Files / Search mode toggle */}
          <div style={{
            display: 'flex', alignItems: 'stretch', flex: '0 0 auto',
            borderBottom: '1px solid var(--wks-border-subtle)',
          }}>
            {(['files', 'search'] as const).map((m) => (
              <span
                key={m}
                onClick={() => setSidebarMode(m)}
                style={{
                  flex: 1, textAlign: 'center', cursor: 'pointer', padding: '4px 0',
                  fontSize: '0.55rem', textTransform: 'uppercase', letterSpacing: '0.05em',
                  color: sidebarMode === m ? 'var(--wks-text-primary)' : 'var(--wks-text-disabled)',
                  background: sidebarMode === m ? 'var(--wks-bg-selected)' : 'transparent',
                  borderBottom: '2px solid ' + (sidebarMode === m ? 'var(--wks-accent, #e6c200)' : 'transparent'),
                }}
              >
                {m}
              </span>
            ))}
          </div>
          {sidebarMode === 'files'
            ? <FileTree root={cwd} activePath={openFile} onOpen={handleTreeOpen} />
            : <SearchPanel cwd={cwd} onOpenMatch={handleSearchOpen} />}
        </div>
      )}

      {/* Editor column */}
      <div style={{ display: 'flex', flexDirection: 'column', flex: 1, minWidth: 0 }}>
        <div style={{
          display: 'flex', alignItems: 'center', gap: 8,
          padding: '4px 10px', flex: '0 0 auto',
          background: 'var(--wks-bg-raised)',
          borderBottom: '1px solid var(--wks-border-subtle)',
          fontSize: '0.65rem', color: 'var(--wks-text-secondary)',
        }}>
          <span style={{ fontFamily: 'var(--wks-mono, ui-monospace, monospace)' }}>{name}</span>
          {dirty && <span title="Unsaved changes" style={{ color: 'var(--wks-accent, #e6c200)' }}>●</span>}
          <div style={{ flex: 1 }} />
          {reloadHint && (
            <span style={{ fontSize: '0.55rem', color: 'var(--wks-text-muted)', fontStyle: 'italic' }}>
              reloaded from disk
            </span>
          )}
          <span style={{ fontSize: '0.55rem', color: 'var(--wks-text-disabled)' }}>
            {saving ? 'Saving…' : dirty ? 'Ctrl+S to save' : status === 'ready' ? 'Saved' : ''}
          </span>
        </div>

        {extChanged && (
          <div style={{
            display: 'flex', alignItems: 'center', gap: 10, flex: '0 0 auto',
            padding: '5px 10px', background: 'var(--wks-accent-bg)',
            borderBottom: '1px solid var(--wks-border-subtle)',
            color: 'var(--wks-text-primary)', fontSize: '0.65rem',
          }}>
            <span>This file changed on disk.</span>
            <div style={{ flex: 1 }} />
            <button
              onClick={handleReloadFromDisk}
              style={{
                cursor: 'pointer', padding: '2px 8px', fontSize: '0.6rem',
                border: '1px solid var(--wks-border-subtle)', borderRadius: 3,
                background: 'var(--wks-bg-base)', color: 'var(--wks-text-primary)',
              }}
            >
              Reload (discard my changes)
            </button>
            <button
              onClick={handleKeepMine}
              style={{
                cursor: 'pointer', padding: '2px 8px', fontSize: '0.6rem',
                border: '1px solid var(--wks-border-subtle)', borderRadius: 3,
                background: 'transparent', color: 'var(--wks-text-secondary)',
              }}
            >
              Keep mine
            </button>
          </div>
        )}

        {!openFile && (
          <div style={{ flex: 1, display: 'grid', placeItems: 'center', color: 'var(--wks-text-disabled)', fontSize: '0.8rem', padding: 16, textAlign: 'center' }}>
            {cwd ? 'Select a file from the tree.' : 'No file open.'}
          </div>
        )}
        {status === 'error' && (
          <div style={{ flex: 1, display: 'grid', placeItems: 'center', color: 'var(--wks-text-muted)', fontSize: '0.78rem', padding: 16, textAlign: 'center' }}>
            Could not open <code>{name}</code>:<br />{error}
          </div>
        )}
        {/* CodeMirror host — hidden (not unmounted) on error/no-file so the view keeps its state. */}
        <div
          ref={hostRef}
          style={{ flex: 1, minHeight: 0, overflow: 'hidden', display: openFile && status !== 'error' ? 'block' : 'none' }}
        />
      </div>
    </div>
  );
};

export default EditorPane;
