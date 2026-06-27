/**
 * Desktop "bus mode" backend — the Electron analogue of the TUI running on the
 * hub bus instead of talking to claudemon directly.
 *
 * The renderer's contract (`window.electronAPI`) is one object, but its methods
 * fall into two planes:
 *
 *   • data / orchestration / observation — spawning + driving agents, sessions,
 *     terminals, config, profiles, library, layouts, files, search. These have
 *     real hub-bus capabilities (served by the brain + main as providers), so in
 *     bus mode they route over the bus exactly like the web build does. One
 *     transport, one set of providers: desktop and web mirror each other.
 *
 *   • host-only desktop concerns — native OS dialogs, plugin management, OS
 *     notifications / ambient focus, window chrome, browser cookie import, and
 *     the MessagePort terminal-exit signal. These have no bus equivalent (they
 *     need the Electron main process), so they stay on the real preload IPC.
 *
 * So this wraps the web (bus) backend and overrides just the host-only slice
 * with the preload's IPC implementations. Toggle the whole thing off with
 * WORKSPACER_DESKTOP_DIRECT=1 (see brainDelegation) to keep pure IPC.
 */

import type { ElectronAPI } from '../types/electron';
import { createWebBackend } from './webBackend';

/**
 * Methods that must keep using the Electron preload (IPC) even in bus mode —
 * they reach native/OS/main-process facilities the bus doesn't expose.
 */
const HOST_ONLY = [
  'setTitleBarOverlay', // Windows native caption-button theming
  'onTerminalExit', // MessagePort exit signal; no bus event for it
  'pickFolder', // native OS folder dialog
  'pickFiles', // native OS file dialog
  'importChromeCookies', // reads the host browser profile
  'listHubPlugins', // plugin registry lives in main
  'installPlugin',
  'removePlugin',
  'setPluginEnabled',
  'getRemoteInfo', // main owns the remote-share/token state
  'onBeforeQuit', // Electron app lifecycle
  'setActiveSession', // OS notification / ambient awareness
  'onFocusAgent',
  'onLibraryChanged', // IPC change event; the bus has no library-change topic
] as const satisfies readonly (keyof ElectronAPI)[];

/**
 * Build the desktop bus-mode backend: the web bus backend pointed at the local
 * hub, with the host-only methods delegated back to the real preload IPC.
 *
 * @param ipc     the preload-provided `window.electronAPI` (IPC transport).
 * @param token   the hub bus bearer token (from `getRemoteInfo`).
 * @param busUrl  the local hub's `ws://…/bus` URL (from `getRemoteInfo`).
 */
export function createBridgedBackend(ipc: ElectronAPI, token: string, busUrl: string): ElectronAPI {
  const bus = createWebBackend(token, busUrl);
  const api = { ...bus } as ElectronAPI;

  // Keep the genuine host platform (the web backend forces 'web', which the UI
  // uses to gate native-only chrome like the Windows titlebar overlay).
  api.platform = ipc.platform;

  for (const key of HOST_ONLY) {
    const fn = ipc[key];
    if (typeof fn === 'function') {
      // Bind to the preload object so its IPC closures keep their `this`.
      (api as Record<string, unknown>)[key] = (fn as (...a: unknown[]) => unknown).bind(ipc);
    }
  }

  return api;
}
