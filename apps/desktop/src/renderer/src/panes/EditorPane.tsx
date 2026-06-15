/**
 * EditorPane — an in-app code editor (Monaco) for a single file.
 *
 * Files are read/written through the app's main-process file backend
 * (window.electronAPI.readFile / writeFile), which works on desktop (IPC) and
 * remote/web (hub fs.read/fs.write capability). The editor is the 'monaco'
 * engine; the 'terminal' engine ($EDITOR in a PTY) is handled in ScrollContainer.
 *
 * No-remount: like the terminal/browser panes, the Monaco editor is created
 * once on mount and kept alive across view-mode changes (ScrollContainer holds
 * stable keys). Only geometry/visibility changes around it — `automaticLayout`
 * resizes the editor when its container does, including when a hidden pane is
 * shown again on agent switch.
 */
import React, { useEffect, useRef, useState } from 'react';
import * as monaco from 'monaco-editor';
import { initVimMode } from 'monaco-vim';
import './../lib/monacoSetup'; // side-effect: wire Monaco's web workers for Vite
import { useConfig } from '../hooks/useConfig';
import { useTheme } from '../hooks/useTheme';
import { isLightTheme, type Theme } from '../themes';

/** Monaco's `defineTheme` only accepts hex colors (no CSS vars), so the editor
 *  is themed from the active theme's ANSI terminal palette — all hex — which
 *  also keeps the editor's look matched to the integrated terminal across every
 *  app theme rather than a generic dark/light preset. Token rule foregrounds
 *  must be hex WITHOUT the leading '#'. */
const THEME_NAME = 'wks';
function defineMonacoTheme(theme: Theme): void {
  const c = theme.terminal;
  const h = (s: string) => s.replace('#', '');
  monaco.editor.defineTheme(THEME_NAME, {
    base: isLightTheme(theme) ? 'vs' : 'vs-dark',
    inherit: true,
    rules: [
      { token: '', foreground: h(c.foreground) },
      { token: 'keyword', foreground: h(c.magenta) },
      { token: 'keyword.control', foreground: h(c.magenta) },
      { token: 'string', foreground: h(c.green) },
      { token: 'string.escape', foreground: h(c.cyan) },
      { token: 'regexp', foreground: h(c.cyan) },
      { token: 'number', foreground: h(c.yellow) },
      { token: 'constant', foreground: h(c.brightYellow) },
      { token: 'comment', foreground: h(c.brightBlack), fontStyle: 'italic' },
      { token: 'type', foreground: h(c.cyan) },
      { token: 'type.identifier', foreground: h(c.cyan) },
      { token: 'namespace', foreground: h(c.cyan) },
      { token: 'function', foreground: h(c.blue) },
      { token: 'identifier', foreground: h(c.foreground) },
      { token: 'variable', foreground: h(c.foreground) },
      { token: 'variable.predefined', foreground: h(c.brightYellow) },
      { token: 'attribute.name', foreground: h(c.brightCyan) },
      { token: 'attribute.value', foreground: h(c.foreground) },
      { token: 'tag', foreground: h(c.red) },
      { token: 'metatag', foreground: h(c.red) },
      { token: 'annotation', foreground: h(c.brightYellow) },
      { token: 'delimiter', foreground: h(c.brightBlack) },
      { token: 'operator', foreground: h(c.brightBlack) },
    ],
    colors: {
      'editor.background': c.background,
      'editor.foreground': c.foreground,
      'editorCursor.foreground': c.cursor,
      'editor.selectionBackground': c.selectionBackground,
      'editor.lineHighlightBackground': c.black,
      'editorLineNumber.foreground': c.brightBlack,
      'editorLineNumber.activeForeground': c.foreground,
      'editorIndentGuide.background': c.black,
      'editorWhitespace.foreground': c.brightBlack,
    },
  });
}

/** Resolve a Monaco language id from a file path via Monaco's language
 *  registry (extension, then exact filename); falls back to plain text. */
function languageForFile(path: string): string {
  const name = basename(path);
  const ext = '.' + (name.split('.').pop() || '').toLowerCase();
  for (const l of monaco.languages.getLanguages()) {
    if (l.extensions?.some((e) => e.toLowerCase() === ext)) return l.id;
    if (l.filenames?.some((f) => f === name)) return l.id;
  }
  return 'plaintext';
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

/** Keep Ctrl/Cmd-S from reaching the app's global "save session" binding. */
function stopSaveBubble(e: KeyboardEvent): void {
  if ((e.ctrlKey || e.metaKey) && (e.key === 's' || e.key === 'S')) {
    e.stopPropagation();
  }
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
  const statusBarRef = useRef<HTMLDivElement>(null);
  const editorRef = useRef<monaco.editor.IStandaloneCodeEditor | null>(null);
  const vimRef = useRef<{ dispose(): void } | null>(null);
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
    const editor = editorRef.current;
    if (!editor || !openFile || saving) return;
    const contents = editor.getValue();
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

  // Create the editor once, load the file, set the right language up front.
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

        defineMonacoTheme(theme);
        const editor = monaco.editor.create(hostRef.current, {
          value: res.contents,
          language: languageForFile(openFile),
          theme: THEME_NAME,
          automaticLayout: true, // resize with the container (incl. show-on-switch)
          minimap: { enabled: false },
          scrollBeyondLastLine: false,
          fontFamily: 'var(--wks-mono, ui-monospace, monospace)',
          fontSize: 13,
          tabSize: 2,
          renderWhitespace: 'selection',
        });
        editorRef.current = editor;

        // Ctrl/Cmd-S saves. Monaco consumes the key, but the app's global
        // "save session" binding listens on document keydown — stop Ctrl-S from
        // bubbling there too.
        editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, () => doSave.current());
        hostRef.current.addEventListener('keydown', stopSaveBubble, true);

        editor.onDidChangeModelContent(() => {
          setDirty(editor.getValue() !== savedRef.current);
        });

        if (vimMode) {
          vimRef.current = initVimMode(editor, statusBarRef.current);
        }
        setStatus('ready');
      })
      .catch((err: unknown) => {
        if (disposed) return;
        setError(String((err as Error)?.message ?? err));
        setStatus('error');
      });

    const host = hostRef.current;
    return () => {
      disposed = true;
      host?.removeEventListener('keydown', stopSaveBubble, true);
      vimRef.current?.dispose();
      vimRef.current = null;
      editorRef.current?.dispose();
      editorRef.current = null;
    };
    // Re-create only when the open file changes (engine/vim changes remount the pane).
  }, [openFile, vimMode]);

  // Re-theme on app theme change without rebuilding the editor: redefining the
  // named theme updates its ANSI-matched palette and re-applies it live.
  useEffect(() => {
    if (!editorRef.current) return;
    defineMonacoTheme(theme);
    monaco.editor.setTheme(THEME_NAME);
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
        {/* Monaco host — hidden (not unmounted) on error/no-file so the editor keeps its state. */}
        <div
          ref={hostRef}
          style={{ flex: 1, minHeight: 0, overflow: 'hidden', display: openFile && status !== 'error' ? 'block' : 'none' }}
        />
        {/* Vim status/command line (`:`, `/`, mode) — only shown in vim mode. */}
        <div
          ref={statusBarRef}
          style={{
            flex: '0 0 auto',
            display: vimMode && openFile && status !== 'error' ? 'block' : 'none',
            padding: '1px 8px',
            fontFamily: 'var(--wks-mono, ui-monospace, monospace)',
            fontSize: '0.65rem',
            color: 'var(--wks-text-secondary)',
            background: 'var(--wks-bg-raised)',
            borderTop: '1px solid var(--wks-border-subtle)',
          }}
        />
      </div>
    </div>
  );
};

export default EditorPane;
