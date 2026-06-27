/**
 * Transport bootstrap. Awaited in `main.tsx` before any React code runs, so the
 * app sees a fully-installed `window.electronAPI` on first render.
 *
 * Three cases:
 *
 *   • Web (no Electron preload): install the hub-bus-backed backend under the
 *     `window.electronAPI` global. The URL is derived from `location` (the hub
 *     serves the app), and the token comes from `?token=` / sessionStorage.
 *
 *   • Desktop, bus mode (default): the preload already populated
 *     `window.electronAPI` over IPC. We mirror the TUI and swap in a bridged
 *     backend that routes the data/orchestration/observation plane over the hub
 *     bus (brain + main as providers), while host-only desktop concerns stay on
 *     IPC. So desktop and web run the same transport against the same providers.
 *
 *   • Desktop, direct mode (WORKSPACER_DESKTOP_DIRECT=1, or an unreachable bus):
 *     leave the preload IPC backend exactly as it was — the prior behavior.
 *
 * Same contract in every case; the unchanged renderer never learns which
 * transport it got.
 */

import { createWebBackend } from './webBackend';
import { createBridgedBackend } from './bridgedBackend';

const TOKEN_KEY = 'hubToken';

/** Web-only: read the hub token from `?token=` (cached so reloads survive). */
function resolveToken(): string {
  const fromQuery = new URLSearchParams(location.search).get('token');
  if (fromQuery) {
    try { sessionStorage.setItem(TOKEN_KEY, fromQuery); } catch { /* ignore */ }
    return fromQuery;
  }
  try { return sessionStorage.getItem(TOKEN_KEY) || ''; } catch { return ''; }
}

export async function installBackend(): Promise<void> {
  // Web build: no contextBridge, so install the bus backend ourselves.
  if (typeof window === 'undefined' || !window.electronAPI) {
    if (typeof window !== 'undefined') window.electronAPI = createWebBackend(resolveToken());
    return;
  }

  // Desktop: the preload gave us the IPC backend. Default to mirroring the TUI
  // by routing through the hub bus; fall back to plain IPC if it's turned off
  // or we can't learn the local bus URL/token.
  const ipc = window.electronAPI;
  try {
    const info = await ipc.getRemoteInfo();
    // Kill switch (WORKSPACER_DESKTOP_DIRECT=1) → main reports desktopBus:false.
    if (!info || info.desktopBus === false) return;
    if (!info.busUrl || !info.token) return; // can't reach the bus — stay on IPC
    window.electronAPI = createBridgedBackend(ipc, info.token, info.busUrl);
    // eslint-disable-next-line no-console
    console.log(`[backend] desktop running on the hub bus (${info.busUrl}); host-only calls stay on IPC.`);
  } catch (err) {
    // getRemoteInfo failed (hub not up yet, etc.) — keep the IPC backend as-is.
    // eslint-disable-next-line no-console
    console.warn('[backend] could not switch desktop to the hub bus; staying on IPC.', err);
  }
}
