import type { HubBusClient } from './hubBusClient';

export interface MachinePowerState {
  canStop: boolean;
  paused: boolean;
  connected: boolean;
  label: string;
  error: string;
  idleMode?: string;
  idle?: {
    quiescent: boolean;
    calmSeconds: number;
    dwellSeconds: number;
    blockers: Array<{ kind: string; detail: string }>;
  };
  wakeUrl?: string;
  waking?: boolean;
}

let client: HubBusClient | undefined;
let state: MachinePowerState = {
  canStop: false,
  paused: false,
  connected: false,
  label: '',
  error: '',
};
const listeners = new Set<() => void>();
function update(patch: Partial<MachinePowerState>): void {
  state = { ...state, ...patch };
  for (const listener of listeners) listener();
}
export const machinePowerSnapshot = (): MachinePowerState => state;
export function subscribeMachinePower(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** Uses the same authenticated connection as the app. No cloud secret reaches
 * the client, and reconnecting is itself the external HTTP wake mechanism. */
let infoKey = '';
export function installMachinePower(connection: HubBusClient, busUrl?: string): void {
  const base = busUrl ?? `${location.protocol === 'https:' ? 'wss:' : 'ws:'}//${location.host}/bus`;
  infoKey = 'wks.machine.wake:' + base;
  try {
    update({ wakeUrl: window.localStorage.getItem(infoKey) || '' });
  } catch {
    /* unavailable */
  }
  client = connection;
  update({ paused: connection.isPowerPaused?.() ?? false });
  connection.onPowerPause?.(() => update({ paused: connection.isPowerPaused(), connected: false }));
  connection.onStatus((connected) => {
    update({ connected });
    if (!connected) return;
    void refreshMachinePower();
  });
}

export async function stopMachine(): Promise<void> {
  if (!client || !state.canStop || !state.connected) return;
  update({ error: '' });
  try {
    await client.call('machine.stop');
    client.pauseForMachineStop();
  } catch (err) {
    // Another client's simultaneous stop can close this RPC before its reply.
    if (!client.isPowerPaused())
      update({ error: err instanceof Error ? err.message : 'Stop failed' });
  }
}

export async function refreshMachinePower(): Promise<void> {
  if (!client || !state.connected) return;
  try {
    const info = await client.call<{
      canStop?: boolean;
      label?: string;
      error?: string;
      wakeUrl?: string;
      idleMode?: string;
      idle?: MachinePowerState['idle'];
    }>('machine.power');
    const wakeUrl = safeWakeURL(info.wakeUrl);
    try {
      if (wakeUrl) window.localStorage.setItem(infoKey, wakeUrl);
      else window.localStorage.removeItem(infoKey);
    } catch {
      /* unavailable */
    }
    update({
      canStop: info.canStop === true,
      label: info.label || 'Server',
      error: info.error || '',
      wakeUrl,
      idleMode: info.idleMode,
      idle: info.idle,
    });
  } catch {
    update({ canStop: false });
  }
}

function safeWakeURL(raw?: string): string {
  if (!raw) return '';
  try {
    const u = new URL(raw);
    return u.protocol === 'https:' && !u.username && !u.password && !u.search && !u.hash
      ? u.href
      : '';
  } catch {
    return '';
  }
}

export async function wakeMachine(): Promise<void> {
  if (state.waking) return;
  update({ error: '', waking: true });
  try {
    const wakeUrl = safeWakeURL(state.wakeUrl);
    if (wakeUrl)
      await fetch(wakeUrl, {
        mode: 'no-cors',
        credentials: 'omit',
        cache: 'no-store',
        // Browsers reject no-cors requests unless redirects are followed.
        redirect: 'follow',
        signal: AbortSignal.timeout(60000),
      });
    client?.resumeMachine();
  } catch {
    update({ error: 'Could not reach the wake endpoint. Try again.' });
  } finally {
    update({ waking: false });
  }
}
