/**
 * Desktop "bus mode" backend — the Electron analogue of the TUI running on the
 * hub bus instead of talking to claudemon directly.
 *
 * The renderer's contract (`window.electronAPI`) is one object, but its methods
 * fall into three planes:
 *
 *   • control + observation — spawning's siblings: driving agents (message,
 *     approve, answer, signal, gate), live snapshots, config, profiles, library,
 *     layouts, files, search. These have real hub-bus capabilities (brain + main
 *     as providers), so they route over the bus exactly like the web build does.
 *     This is the mirror that matters: desktop and web drive + observe agents
 *     through the identical path.
 *
 *   • local terminal byte transport — the raw PTY byte streams and the lifecycle
 *     that establishes them (create/spawn/attach/detach, output, write, resize,
 *     close). On the web these MUST cross the network as `pty.bytes` events over
 *     the bus (base64-framed through the hub) — there's no MessagePort across a
 *     wire. On the desktop everything is local, so we keep these on the preload
 *     IPC, which streams bytes over a direct MessagePort: no hub hop, no base64,
 *     no coalescing latency. The bus PTY path is therefore web-only.
 *
 *   • host-only desktop concerns — native OS dialogs, plugin management, OS
 *     notifications / ambient focus, window chrome, browser cookie import, and
 *     the terminal-exit signal. These have no bus equivalent (they need the
 *     Electron main process), so they stay on the real preload IPC.
 *
 * So this wraps the web (bus) backend and overrides the local-terminal and
 * host-only slices with the preload's IPC implementations. Toggle the whole
 * thing off with WORKSPACER_DESKTOP_DIRECT=1 (see brainDelegation) for pure IPC.
 */

import type { ElectronAPI } from '../types/electron';
import { createWebBackend } from './webBackend';

/**
 * Local terminal byte transport — kept on the preload IPC so the desktop streams
 * PTY bytes over a direct MessagePort instead of the bus's `pty.bytes` events
 * (which exist for the web, where the bytes must cross a network). This is the
 * whole create/spawn → attach → stream/write/resize → close lifecycle, since the
 * MessagePort a viewer reads from is established by the IPC spawn/attach and torn
 * down by the IPC close/detach; splitting it across transports would orphan the
 * port. Driving (message/approve/answer/signal/gate) and observation
 * (snapshots) are NOT here — they carry no bytes and stay on the bus mirror.
 */
export const LOCAL_TERMINAL = [
  'createTerminal', // shell PTY spawn → main delivers its MessagePort
  'writeTerminal',
  'resizeTerminal',
  'closeTerminal',
  'onTerminalOutput',
  'spawnClaude', // claude session spawn → main delivers its byte MessagePort
  'attachClaude', // viewer pane → its own MessagePort
  'detachClaude',
  'onClaudeOutput',
  'claudeWrite',
  'claudeResize',
  'claudeClose',
] as const satisfies readonly (keyof ElectronAPI)[];

/**
 * Host-only desktop concerns that must keep using the Electron preload (IPC) —
 * they reach native/OS/main-process facilities the bus doesn't expose.
 */
export const HOST_ONLY = [
  'setTitleBarOverlay', // Windows native caption-button theming
  'onTerminalExit', // MessagePort exit signal; no bus event for it
  'pickFolder', // native OS folder dialog
  'pickFiles', // native OS file dialog
  'importChromeCookies', // reads the host browser profile
  'toolsStatus', // external-tool PATH scan runs on the host
  'notesList', // notes live in the host config dir
  'notesSave',
  'notesDelete',
  'listHubPlugins', // plugin registry lives in main
  'installPlugin',
  'inspectPlugin', // pre-install manifest preview via the hub's guarded route
  'listExamplePlugins', // bundled-example catalog lives in main
  'installExamplePlugin',
  'removePlugin',
  'setPluginEnabled',
  'pluginPaneToken', // trusted-host mint via the hub's guarded route
  'revokePluginPaneToken',
  'getPluginSettings', // host-persisted plugin settings
  'setPluginSettings',
  'onPluginSettingsChanged',
  'getRemoteInfo', // main owns the remote-share/token state
  'setRemoteShare', // toggling host sharing is a host action (restarts the hub)
  'onBeforeQuit', // Electron app lifecycle
  'setActiveSession', // OS notification / ambient awareness
  'onFocusAgent',
  'onSystemNotice', // main-process daemon/startup notices; IPC-only push
  'openLogsFolder', // opens the host's logs dir in its file manager
  'installCli', // installs the host's bundled workspacer CLI onto the host PATH
  'pricingGetRates', // model-rate table + overrides read from the host rates file
  'pricingSaveOverrides', // writes the host's ~/.workspacer/model-rates.json
  'onLibraryChanged', // IPC change event; the bus has no library-change topic
  'worktreeInfo', // worktree ops shell out to git on the host
  'worktreeCreate',
  'updatesGetStatus', // in-app updates are a desktop-shell (electron-updater) concern
  'updatesCheck',
  'updatesInstall',
  'onUpdateStatus',
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

  // Delegate the local-terminal and host-only slices back to the preload IPC;
  // everything else stays on the bus (the web backend's implementation).
  for (const key of [...LOCAL_TERMINAL, ...HOST_ONLY]) {
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
