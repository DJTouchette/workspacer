/**
 * Transport bootstrap. Imported first in `main.tsx`, before any React code runs.
 *
 * In Electron the preload has already populated `window.electronAPI`, so this is
 * a no-op and the desktop path is completely untouched. In a browser there is no
 * preload, so we install the hub-bus-backed implementation under the same global
 * — giving the unchanged renderer a working backend. Same contract, two
 * providers (contextBridge on desktop, this on web).
 *
 * The hub token is read from `?token=` (and cached in sessionStorage so it
 * survives reloads), mirroring `remote.html`'s gate.
 */

import { createWebBackend } from './webBackend';

const TOKEN_KEY = 'hubToken';

function resolveToken(): string {
  const fromQuery = new URLSearchParams(location.search).get('token');
  if (fromQuery) {
    try { sessionStorage.setItem(TOKEN_KEY, fromQuery); } catch { /* ignore */ }
    return fromQuery;
  }
  try { return sessionStorage.getItem(TOKEN_KEY) || ''; } catch { return ''; }
}

export function installBackend(): void {
  // Desktop: the contextBridge already provided the real API.
  if (typeof window !== 'undefined' && window.electronAPI) return;

  const token = resolveToken();
  window.electronAPI = createWebBackend(token);
}

installBackend();
