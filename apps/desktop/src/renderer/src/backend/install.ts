/**
 * Transport bootstrap. Awaited in `main.tsx` before any React code runs, so the
 * app sees a fully-installed `window.electronAPI` on first render.
 *
 * Four cases:
 *
 *   • Web (no Electron preload): install the hub-bus-backed backend under the
 *     `window.electronAPI` global. The URL is derived from `location` (the hub
 *     serves the app), and the token comes from `?token=` / sessionStorage.
 *
 *   • Desktop, remote-client mode ("Connect to remote server"): main reports a
 *     configured remote hub (getRemoteInfo().remoteClient) and spawned no local
 *     daemons. The renderer boots the web backend dialed at the REMOTE hub —
 *     what a browser gets at that server's /app URL, inside the shell — with
 *     only host-shell concerns on IPC (see remoteBackend).
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
import { createRemoteBackend } from './remoteBackend';

const TOKEN_KEY = 'hubToken';

/** Web-only: read the hub token from `?token=` (cached so reloads survive). */
function resolveToken(): string {
  const fromQuery = new URLSearchParams(location.search).get('token');
  if (fromQuery) {
    try {
      sessionStorage.setItem(TOKEN_KEY, fromQuery);
    } catch {
      /* ignore */
    }
    return fromQuery;
  }
  try {
    return sessionStorage.getItem(TOKEN_KEY) || '';
  } catch {
    return '';
  }
}

/** The slice of getRemoteInfo() the desktop transport decision reads. */
export interface BackendModeInfo {
  desktopBus?: boolean;
  busUrl?: string;
  token?: string;
  remoteClient?: { busUrl: string; token: string } | null;
}

export type BackendMode = 'ipc' | 'bridged' | 'remote';

/**
 * Backoff for re-asking main which transport to boot. `getRemoteInfo()` can
 * reject transiently — the renderer's first paint can beat IPC handler
 * registration ("No handler registered for…"), and main does async probe work
 * behind that call. One failed ask used to leave the app on plain IPC forever.
 *
 * That default is only harmless in LOCAL mode. In remote-client mode main
 * deliberately spawned no claudemon/hub/brain, so plain IPC has nothing behind
 * it: the user gets a window that looks fine and does nothing. Since the
 * renderer cannot tell those two cases apart without this very answer, retry
 * rather than guess. ~2s total, all of it before first paint.
 */
const REMOTE_INFO_BACKOFF_MS = [50, 150, 300, 600, 1000];

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/**
 * `ipc.getRemoteInfo()` with bounded retries. Rejects with the LAST error once
 * the backoff is exhausted, so the caller can report why rather than silently
 * degrade. Generic in the payload so the caller keeps the full RemoteInfo type
 * (BackendModeInfo is only the slice `selectBackendMode` reads). Exported for
 * tests.
 */
export async function getRemoteInfoWithRetry<T>(
  getRemoteInfo: () => Promise<T>,
  backoff: number[] = REMOTE_INFO_BACKOFF_MS,
): Promise<T> {
  let lastErr: unknown;
  for (let attempt = 0; attempt <= backoff.length; attempt++) {
    try {
      return await getRemoteInfo();
    } catch (err) {
      lastErr = err;
      if (attempt < backoff.length) await sleep(backoff[attempt]);
    }
  }
  throw lastErr;
}

/**
 * Pick the desktop transport from main's remote info. Pure — exported for
 * tests. Remote-client mode wins outright: when a remote server is configured,
 * main spawned no local daemons, so neither the local bus (bridged) nor local
 * IPC data paths would have anything to talk to.
 */
export function selectBackendMode(info: BackendModeInfo | null | undefined): BackendMode {
  if (info?.remoteClient?.busUrl) return 'remote';
  // Kill switch (WORKSPACER_DESKTOP_DIRECT=1) → main reports desktopBus:false.
  if (!info || info.desktopBus === false) return 'ipc';
  if (!info.busUrl || !info.token) return 'ipc'; // can't reach the bus — stay on IPC
  return 'bridged';
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
    const info = await getRemoteInfoWithRetry(() => ipc.getRemoteInfo());
    switch (selectBackendMode(info)) {
      case 'remote': {
        const rc = info.remoteClient!;
        window.electronAPI = createRemoteBackend(ipc, rc.token ?? '', rc.busUrl);
        // eslint-disable-next-line no-console
        console.log(
          `[backend] remote-client mode: running against ${rc.busUrl}; host-shell calls stay on IPC.`,
        );
        return;
      }
      case 'bridged':
        window.electronAPI = createBridgedBackend(ipc, info.token, info.busUrl);
        // eslint-disable-next-line no-console
        console.log(
          `[backend] desktop running on the hub bus (${info.busUrl}); host-only calls stay on IPC.`,
        );
        return;
      case 'ipc':
        return;
    }
  } catch (err) {
    // Every retry failed. Staying on the preload IPC backend is correct for a
    // local install and BROKEN for a remote-client one (no local daemons were
    // spawned) — and we still can't tell which this is. Say so loudly: a silent
    // warning here is what made the stranded case look like the app "just not
    // working" instead of a transport that never resolved.
    // eslint-disable-next-line no-console
    console.error(
      '[backend] could not read the transport setting from main after retries; staying on IPC. ' +
        'If this desktop is configured to connect to a remote workspacer server, this window will ' +
        'NOT work — reopen "Connect to Server…" and reconnect, or restart the app.',
      err,
    );
  }
}
