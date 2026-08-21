import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

const h = vi.hoisted(() => ({
  dir: '',
  config: {} as Record<string, unknown>,
  listeners: [] as Array<(cfg: unknown) => void>,
}));
// One mock serves both importers: remoteTokens (getConfigDir → the temp token
// file) and fullAccessGrants (getConfig / onChange → the mutable test config).
vi.mock('./configService', () => ({
  getConfigDir: () => h.dir,
  configService: {
    getConfig: () => h.config,
    onChange: (cb: (cfg: unknown) => void) => {
      h.listeners.push(cb);
      return () => {};
    },
  },
}));

import { mintSessionFacadeToken } from './remoteTokens';
import {
  managerFullAccessFromConfig,
  supervisorFullAccessFromConfig,
  reconcileFullAccessGrants,
  startFullAccessGrantSync,
} from './fullAccessGrants';

const rawTokens = () =>
  JSON.parse(fs.readFileSync(path.join(h.dir, 'tokens.json'), 'utf-8')) as Array<
    Record<string, unknown>
  >;

beforeEach(() => {
  h.dir = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-fullaccess-'));
  h.config = {};
});

afterEach(() => {
  fs.rmSync(h.dir, { recursive: true, force: true });
});

describe('full-access grant formulas (single source for mint + reconcile)', () => {
  it('manager: agents.fleetFullAccess OR any per-project yolo flag', () => {
    expect(managerFullAccessFromConfig()).toBe(false);

    h.config = { agents: { fleetFullAccess: true } };
    expect(managerFullAccessFromConfig()).toBe(true);

    h.config = { projects: { '/a': {}, '/b': { yolo: true } } };
    expect(managerFullAccessFromConfig()).toBe(true);

    h.config = { agents: { fleetFullAccess: false }, projects: { '/a': { yolo: false } } };
    expect(managerFullAccessFromConfig()).toBe(false);
  });

  it('supervisor: supervisor.fullAccess only', () => {
    expect(supervisorFullAccessFromConfig()).toBe(false);
    h.config = { supervisor: { fullAccess: true } };
    expect(supervisorFullAccessFromConfig()).toBe(true);
    // The fleet flag does not bleed into the supervisor grant (or vice versa).
    h.config = { agents: { fleetFullAccess: true } };
    expect(supervisorFullAccessFromConfig()).toBe(false);
  });
});

describe('config flip → live token update', () => {
  it('a flag flip updates exactly the role-tagged session tokens, both directions', () => {
    const mgr = mintSessionFacadeToken(
      'mgr-1',
      'operator',
      undefined,
      ['default'],
      true,
      'manager',
    );
    const sup = mintSessionFacadeToken(
      'sup-1',
      'operator',
      undefined,
      undefined,
      true,
      'supervisor',
    );
    const worker = mintSessionFacadeToken('wkr-1', 'view');

    // Flags now OFF (revocation must be live): both role tokens lose the grant.
    h.config = {};
    expect(reconcileFullAccessGrants()).toBe(2);
    expect(rawTokens().find((r) => r.token === mgr.token)).not.toHaveProperty('yoloAllowed');
    expect(rawTokens().find((r) => r.token === sup.token)).not.toHaveProperty('yoloAllowed');

    // Manager flag back ON (off→on is live too); the worker stays untouched.
    h.config = { agents: { fleetFullAccess: true } };
    expect(reconcileFullAccessGrants()).toBe(1);
    expect(rawTokens().find((r) => r.token === mgr.token)).toMatchObject({ yoloAllowed: true });
    expect(rawTokens().find((r) => r.token === sup.token)).not.toHaveProperty('yoloAllowed');
    expect(rawTokens().find((r) => r.token === worker.token)).not.toHaveProperty('yoloAllowed');

    // In line → nothing to do.
    expect(reconcileFullAccessGrants()).toBe(0);
  });

  it('startFullAccessGrantSync reconciles on every config change notification', () => {
    const mgr = mintSessionFacadeToken(
      'mgr-1',
      'operator',
      undefined,
      ['default'],
      true,
      'manager',
    );

    startFullAccessGrantSync();
    expect(h.listeners.length).toBeGreaterThan(0);

    // The user turns fleet full access off: the change event alone must strip
    // the live manager token's grant (the facade re-reads it per request, so
    // this IS the live revocation).
    h.config = {};
    for (const cb of h.listeners) cb(h.config);
    expect(rawTokens().find((r) => r.token === mgr.token)).not.toHaveProperty('yoloAllowed');

    // …and back on.
    h.config = { projects: { '/repo': { yolo: true } } };
    for (const cb of h.listeners) cb(h.config);
    expect(rawTokens().find((r) => r.token === mgr.token)).toMatchObject({ yoloAllowed: true });
  });
});
