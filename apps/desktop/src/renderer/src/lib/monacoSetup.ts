/**
 * Monaco web-worker wiring for Vite.
 *
 * Monaco runs its language services (TS/JS, JSON, CSS, HTML) in web workers.
 * Vite's `?worker` imports bundle each worker as its own chunk and resolve the
 * URL relative to the build's `base` — so this works for BOTH build targets:
 * the Electron renderer (base './', loaded over file://) and the hub web app
 * (base '/app/'). Import this module for its side effect before any
 * monaco.editor.create() call (EditorPane does so at the top of the file).
 */
import editorWorker from 'monaco-editor/esm/vs/editor/editor.worker?worker';
import jsonWorker from 'monaco-editor/esm/vs/language/json/json.worker?worker';
import cssWorker from 'monaco-editor/esm/vs/language/css/css.worker?worker';
import htmlWorker from 'monaco-editor/esm/vs/language/html/html.worker?worker';
import tsWorker from 'monaco-editor/esm/vs/language/typescript/ts.worker?worker';

(self as unknown as { MonacoEnvironment: unknown }).MonacoEnvironment = {
  getWorker(_workerId: string, label: string): Worker {
    switch (label) {
      case 'json':
        return new jsonWorker();
      case 'css':
      case 'scss':
      case 'less':
        return new cssWorker();
      case 'html':
      case 'handlebars':
      case 'razor':
        return new htmlWorker();
      case 'typescript':
      case 'javascript':
        return new tsWorker();
      default:
        return new editorWorker();
    }
  },
};
