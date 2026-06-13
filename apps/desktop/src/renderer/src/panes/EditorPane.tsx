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

function basename(p: string): string {
  return p.split(/[\\/]/).pop() || p;
}

const EditorPane: React.FC<EditorPaneProps> = ({ filePath }) => {
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

  // Re-theme on app theme change without rebuilding the editor. The chrome uses
  // --wks-* vars (updates for free); this swaps in the new theme's ANSI-matched
  // syntax palette.
  useEffect(() => {
    viewRef.current?.dispatch({
      effects: themeCompartment.current.reconfigure(themeExtensions(theme)),
    });
  }, [theme]);

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
