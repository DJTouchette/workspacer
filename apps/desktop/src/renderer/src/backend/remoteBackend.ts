/**
 * "Connect to remote server" backend — the Electron shell running as a pure
 * client of an external workspacer server (`workspacer serve` on another
 * machine). This is exactly what a browser gets at that server's /app URL,
 * but inside the shell: the web (bus) backend dialed at the REMOTE hub.
 *
 * Unlike the desktop bus mode (bridgedBackend), the local-terminal slice must
 * NOT fall back to the preload IPC here — there is no local claudemon in this
 * mode (main skips daemon spawn entirely, see main/index.ts), and the sessions
 * live on the remote host anyway. PTY bytes therefore ride the bus as
 * `pty.bytes.*` events, just like the web client.
 *
 * Only a minimal host-shell slice stays on the real preload IPC: things that
 * concern THIS window/process rather than any server — window chrome, the quit
 * handshake, opening external URLs/logs, and the remote-connection settings
 * themselves (getRemoteInfo / setRemoteServer / appRelaunch), which must reach
 * main so the user can disconnect and relaunch back into local mode.
 */

import type { ElectronAPI } from '../types/electron';
import { createWebBackend } from './webBackend';

/** Host-shell methods that must keep using the Electron preload (IPC). */
const REMOTE_HOST_ONLY = [
  'setTitleBarOverlay', // Windows native caption-button theming
  'onBeforeQuit', // Electron app lifecycle (quit-save handshake)
  'onSystemNotice', // main-process notices (e.g. relaunch/setting errors)
  'openExternalUrl', // open http(s) links in the host's default browser
  'openLogsFolder', // the host's logs dir
  'getRemoteInfo', // main owns the remote-client/adopted state
  'setRemoteServer', // disconnect persists on the host…
  'appRelaunch', // …and relaunching applies it
] as const satisfies readonly (keyof ElectronAPI)[];

/**
 * Build the remote-client backend: the web bus backend pointed at the remote
 * hub, with the host-shell slice delegated back to the real preload IPC.
 */
export function createRemoteBackend(ipc: ElectronAPI, token: string, busUrl: string): ElectronAPI {
  const bus = createWebBackend(token, busUrl);
  const api = { ...bus } as ElectronAPI;

  // Keep the genuine host platform (the web backend forces 'web', which the UI
  // uses to gate native-only chrome like the Windows titlebar overlay).
  api.platform = ipc.platform;

  for (const key of REMOTE_HOST_ONLY) {
    const fn = ipc[key];
    if (typeof fn === 'function') {
      // Bind to the preload object so its IPC closures keep their `this`.
      (api as unknown as Record<string, unknown>)[key] = (fn as (...a: unknown[]) => unknown).bind(
        ipc,
      );
    }
  }

  return api;
}
