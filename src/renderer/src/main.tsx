import React from 'react'
import ReactDOM from 'react-dom/client'
import App from './App'

// Load Nerd Fonts BEFORE React renders — xterm.js canvas won't redraw after initial paint
// This must run at module level so the promise is available before any component mounts
;(window as any).__fontsReady = (async () => {
  try {
    const fonts = await (window as any).electronAPI?.getNerdFonts?.();
    if (!fonts?.length) return;
    for (const f of fonts) {
      const buf = f.data instanceof ArrayBuffer ? f.data
        : f.data?.buffer instanceof ArrayBuffer ? f.data.buffer.slice(f.data.byteOffset, f.data.byteOffset + f.data.byteLength)
        : f.data;
      const names = [f.family];
      const generic = f.family.replace(/NL\s*/g, '');
      if (generic !== f.family) names.push(generic);
      for (const name of names) {
        try {
          const face = new FontFace(name, buf);
          await face.load();
          document.fonts.add(face);
          console.log(`[Fonts] loaded: "${name}"`);
        } catch (err) {
          console.warn(`[Fonts] failed "${name}":`, err);
        }
      }
    }
  } catch (err) {
    console.warn('[Fonts] discovery failed:', err);
  }
})();

ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
)
