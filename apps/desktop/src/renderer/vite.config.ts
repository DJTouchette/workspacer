import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

export default defineConfig({
  plugins: [react()],
  base: './',
  build: {
    outDir: '../../dist/renderer',
    emptyOutDir: true,
    rollupOptions: {
      output: {
        manualChunks: {
          // Split heavy dependencies into separate chunks for better caching
          react: ['react', 'react-dom'],
          xterm: ['@xterm/xterm', '@xterm/addon-fit', '@xterm/addon-web-fonts'],
          codemirror: ['@codemirror/commands', '@codemirror/language', '@codemirror/language-data', '@codemirror/state', '@codemirror/theme-one-dark', '@codemirror/view', '@lezer/highlight', '@replit/codemirror-vim', 'codemirror'],
          shiki: ['@shikijs/core', '@shikijs/engine-javascript'],
        },
      },
    },
  },
  server: {
    port: 5173,
  },
});
