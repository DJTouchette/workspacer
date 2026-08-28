import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

const h = vi.hoisted(() => ({
  dir: '',
  config: {} as Record<string, unknown>,
  listeners: [] as Array<(cfg: unknown) => void>,
  posted: [] as Array<Record<string, unknown>>,
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

vi.mock('./agentNotifier', () => ({
  agentNotifier: {
    postInApp: (n: Record<string, unknown>) => {
      h.posted.push(n);
    },
  },
}));

import { mintSessionFacadeToken } from './remoteTokens';
import {
  managerFullAccessFromConfig,
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
  h.posted = [];
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
    const worker = mintSessionFacadeToken('wkr-1', 'view');

    // Flag now OFF (revocation must be live): the role token loses the grant.
    h.config = {};
    expect(reconcileFullAccessGrants()).toBe(1);
    expect(rawTokens().find((r) => r.token === mgr.token)).not.toHaveProperty('yoloAllowed');

    // Manager flag back ON (off→on is live too); the worker stays untouched.
    h.config = { agents: { fleetFullAccess: true } };
    expect(reconcileFullAccessGrants()).toBe(1);
    expect(rawTokens().find((r) => r.token === mgr.token)).toMatchObject({ yoloAllowed: true });
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

describe('telling the user what a flip did and did NOT do', () => {
  const mgr = () =>
    mintSessionFacadeToken('mgr-1', 'operator', undefined, ['default'], true, 'manager');

  it("announces only when a LIVE session's grant actually moved", () => {
    // No role-tagged token at all: toggling the flag is not news.
    h.config = { agents: { fleetFullAccess: true } };
    reconcileFullAccessGrants(true);
    expect(h.posted).toHaveLength(0);

    mgr();
    // Already in line → still nothing.
    reconcileFullAccessGrants(true);
    expect(h.posted).toHaveLength(0);

    h.config = {};
    reconcileFullAccessGrants(true);
    expect(h.posted).toHaveLength(1);
  });

  it('the boot reconcile stays silent — catching up on a closed-app flip is not news', () => {
    mgr();
    h.config = {};
    expect(reconcileFullAccessGrants()).toBe(1);
    expect(h.posted).toHaveLength(0);
  });

  it("says both halves: dispatches change live, the session's own bypass does not", () => {
    mgr();
    h.config = {};
    reconcileFullAccessGrants(true);

    const off = h.posted[0];
    expect(off.sessionId).toBe('mgr-1');
    expect(off.title).toContain('Fleet Manager');
    // The whole point of the notice: the part that CANNOT apply live.
    expect(String(off.body)).toMatch(/respawn/i);

    h.config = { agents: { fleetFullAccess: true } };
    reconcileFullAccessGrants(true);
    const on = h.posted[1];
    expect(String(on.title)).toContain('on');
    expect(String(on.body)).toMatch(/dispatch/i);
    expect(String(on.body)).toMatch(/respawn/i);
    // Same key both ways, so flipping back and forth replaces rather than
    // stacking contradictory notes.
    expect(on.key).toBe(off.key);
  });
});
