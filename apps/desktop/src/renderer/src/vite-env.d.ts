/// <reference types="vite/client" />

// monaco-vim ships no type declarations. initVimMode(editor, statusBarEl?)
// attaches Vim keybindings to a Monaco editor and returns a disposable.
declare module 'monaco-vim' {
  import type { editor } from 'monaco-editor';
  export function initVimMode(
    editor: editor.IStandaloneCodeEditor,
    statusbarNode?: HTMLElement | null,
  ): { dispose(): void };
}
