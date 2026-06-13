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
import { oneDark } from '@codemirror/theme-one-dark';
import { vim } from '@replit/codemirror-vim';
import { useConfig } from '../hooks/useConfig';

interface EditorPaneProps {
  paneId: string;
  title: string;
  isActive: boolean;
  filePath?: string;
  cwd?: string;
}

type Status = 'idle' | 'loading' | 'ready' | 'error';

function basename(p: string): string {
  return p.split(/[\\/]/).pop() || p;
}

const EditorPane: React.FC<EditorPaneProps> = ({ filePath }) => {
  const { config } = useConfig();
  const vimMode = config.keybindings?.mode === 'vim';

  const hostRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const langCompartment = useRef(new Compartment());
  // Latest persisted contents, to compute the dirty flag and to skip no-op saves.
  const savedRef = useRef<string>('');

  const [status, setStatus] = useState<Status>('idle');
  const [error, setError] = useState<string>('');
  const [dirty, setDirty] = useState(false);
  const [saving, setSaving] = useState(false);

  const doSave = useRef<() => void>(() => {});
  doSave.current = () => {
    const view = viewRef.current;
    if (!view || !filePath || saving) return;
    const contents = view.state.doc.toString();
    if (contents === savedRef.current) return; // nothing changed
    setSaving(true);
    window.electronAPI
      .writeFile(filePath, contents)
      .then(() => {
        savedRef.current = contents;
        setDirty(false);
      })
      .catch((err: unknown) => setError(String((err as Error)?.message ?? err)))
      .finally(() => setSaving(false));
  };

  // Create the editor once, load the file, lazily swap in the right language.
  useEffect(() => {
    if (!hostRef.current || !filePath) return;
    let disposed = false;

    setStatus('loading');
    setError('');
    window.electronAPI
      .readFile(filePath)
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
          oneDark,
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
          EditorView.theme({ '&': { height: '100%' }, '.cm-scroller': { overflow: 'auto' } }),
        ];

        viewRef.current = new EditorView({
          state: EditorState.create({ doc: res.contents, extensions }),
          parent: hostRef.current,
        });
        setStatus('ready');

        // Resolve the language for this extension and reconfigure when ready.
        const desc = languages.find((l) => l.extensions.includes(basename(filePath).split('.').pop() || ''));
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
    // Re-create only when the file changes (engine/vim changes remount the pane).
  }, [filePath, vimMode]);

  const name = filePath ? basename(filePath) : 'No file';

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%', background: 'var(--wks-bg-base)' }}>
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

      {!filePath && (
        <div style={{ flex: 1, display: 'grid', placeItems: 'center', color: 'var(--wks-text-disabled)', fontSize: '0.8rem' }}>
          No file open.
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
        style={{ flex: 1, minHeight: 0, overflow: 'hidden', display: filePath && status !== 'error' ? 'block' : 'none' }}
      />
    </div>
  );
};

export default EditorPane;
