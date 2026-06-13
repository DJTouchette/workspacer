import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// Web build of the renderer — the SAME app as the Electron build, served by the
// hub under /app/ for full remote parity (see backend/install.ts, which installs
// the hub-bus-backed window.electronAPI when there's no Electron preload).
//
// Differences from vite.config.ts:
//   - base '/app/'  : assets are served under the hub's /app/ subtree, not root.
//   - outDir dist/web: kept separate from the Electron bundle (dist/renderer).
export default defineConfig({
  plugins: [react()],
  base: '/app/',
  build: {
    outDir: '../../dist/web',
    emptyOutDir: true,
    rollupOptions: {
      output: {
        manualChunks: {
          react: ['react', 'react-dom'],
          xterm: ['@xterm/xterm', '@xterm/addon-fit', '@xterm/addon-web-fonts', '@xterm/addon-webgl'],
        },
      },
    },
  },
});
