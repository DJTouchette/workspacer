import { beforeEach, describe, expect, it, vi } from 'vitest';
const state = vi.hoisted(() => ({ enabled: false, rows: {} as Record<string, any> }));
vi.mock('./configService', () => ({
  configService: { getConfig: () => ({ agents: { fleetFullAccess: state.enabled } }) },
}));
vi.mock('./claudeSessionStore', () => ({
  claudeSessionStore: { getSnapshot: (id: string) => state.rows[id] ?? null },
}));
import { fleetSkipsPermissions } from './fleetPermissions';
beforeEach(() => {
  state.enabled = false;
  state.rows = {};
});
describe('Fleet provider approval preference', () => {
  it('uses current settings for managers and their recorded descendants only', () => {
    state.rows = {
      manager: { isWakeTarget: true },
      worker: { parentSessionId: 'manager' },
      ordinary: {},
    };
    expect(fleetSkipsPermissions({ manager: true })).toBe(false);
    state.enabled = true;
    expect(fleetSkipsPermissions({ manager: true })).toBe(true);
    expect(fleetSkipsPermissions({ parentSessionId: 'manager' })).toBe(true);
    expect(fleetSkipsPermissions({ parentSessionId: 'worker' })).toBe(true);
    expect(fleetSkipsPermissions({ parentSessionId: 'ordinary' })).toBe(false);
    expect(fleetSkipsPermissions({ parentSessionId: 'missing' })).toBe(false);
    expect(fleetSkipsPermissions({})).toBe(false);
    state.enabled = false;
    expect(fleetSkipsPermissions({ parentSessionId: 'worker' })).toBe(false);
  });
  it('stops at cycles and recognizes restored manager snapshots', () => {
    state.enabled = true;
    state.rows = {
      a: { parentSessionId: 'b' },
      b: { parentSessionId: 'a' },
      restored: { isFleetManager: true },
    };
    expect(fleetSkipsPermissions({ parentSessionId: 'a' })).toBe(false);
    expect(fleetSkipsPermissions({ parentSessionId: 'restored' })).toBe(true);
  });
});
