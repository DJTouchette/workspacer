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
import { languages } from '@codemirror/language-data';
import { syntaxHighlighting, HighlightStyle } from '@codemirror/language';
import { tags as t } from '@lezer/highlight';
import { vim } from '@replit/codemirror-vim';
import { useConfig } from '../hooks/useConfig';
import { useTheme } from '../hooks/useTheme';
import { isLightTheme, type Theme } from '../themes';

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

/** File-tree sidebar rooted at the agent's cwd. Refresh re-keys the root. */
const FileTree: React.FC<{ root: string; activePath?: string; onOpen: (p: string) => void }> = ({ root, activePath, onOpen }) => {
  const [reloadKey, setReloadKey] = useState(0);
  return (
    <div style={{
      width: 220, flex: '0 0 auto', overflow: 'auto',
      borderRight: '1px solid var(--wks-border-subtle)', background: 'var(--wks-bg-raised)',
      fontFamily: 'var(--wks-mono, ui-monospace, monospace)', fontSize: '0.7rem', paddingBottom: 8,
    }}>
      <div style={{
        display: 'flex', alignItems: 'center', padding: '4px 8px', position: 'sticky', top: 0,
        background: 'var(--wks-bg-raised)', borderBottom: '1px solid var(--wks-border-subtle)',
        color: 'var(--wks-text-disabled)', fontSize: '0.55rem', textTransform: 'uppercase', letterSpacing: '0.05em',
      }}>
        <span>Files</span>
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
  const vimMode = config.keybindings?.mode === 'vim';

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
    window.electronAPI
      .readFile(openFile)
      .then((res: { contents: string }) => {
        if (disposed || !hostRef.current) return;
        savedRef.current = res.contents;

        const saveKeys = keymap.of([
          { key: 'Mod-s', preventDefault: true, run: () => { doSave.current(); return true; } },
          indentWithTab,
        ]);
        const extensions = [
          ...(vimMode ? [vim()] : []), // vim must come first
          basicSetup,
          langCompartment.current.of([]),
          themeCompartment.current.of(themeExtensions(theme)),
          saveKeys,
          // Stop Ctrl-S bubbling to the app's global "save session" binding.
          EditorView.domEventHandlers({
            keydown: (e) => {
              if ((e.ctrlKey || e.metaKey) && (e.key === 's' || e.key === 'S')) {
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

  const name = openFile ? basename(openFile) : 'No file';

  // Switch the open file from the tree, saving any pending edits to the old one first.
  const handleTreeOpen = (p: string) => {
    if (p === openFile) return;
    doSave.current();
    setOpenFile(p);
  };

  return (
    <div style={{ display: 'flex', height: '100%', background: 'var(--wks-bg-base)' }}>
      {cwd && <FileTree root={cwd} activePath={openFile} onOpen={handleTreeOpen} />}

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
          <span style={{ fontSize: '0.55rem', color: 'var(--wks-text-disabled)' }}>
            {saving ? 'Saving…' : dirty ? 'Ctrl+S to save' : status === 'ready' ? 'Saved' : ''}
          </span>
        </div>

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
