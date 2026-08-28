/**
 * The regression this file exists for: the `claude:spawn` IPC handler's managed
 * branch hand-copied option fields and silently dropped `manager` and
 * `fleetFullAccess`, so a Fleet Manager on Codex came up unflagged — never
 * marked isSupervisor (no worker-finished wake could route to it) and with a
 * facade token minted without the profilesAllowed / yolo grants.
 *
 * The completeness test below is the real guard: it walks EVERY field of the
 * spawn request and asserts each one either reaches the managed spawn options
 * or is declared unsupported/derived with a reason. A new spawn option cannot
 * be added and quietly ignored — SPAWN_REQUEST_FIELDS fails to compile until it
 * is classified, and this test fails until the classification is honest.
 */
import { describe, it, expect, vi } from 'vitest';
import {
  SPAWN_REQUEST_FIELDS,
  managedOptionsFromRequest,
  explainUnsupportedManagedOptions,
  type AgentSpawnRequest,
} from './managedSpawnOptions';

/** A request with every field set to a distinguishable non-default value. */
const FULL_REQUEST: Required<AgentSpawnRequest> = {
  cwd: '/home/u/Work/proj',
  provider: 'codex',
  transport: 'stream',
  profileId: 'work-account',
  manager: true,
  fleetFullAccess: true,
  model: 'gpt-5.5',
  effort: 'high',
  permissionMode: 'yolo',
  skipPermissions: true,
  resumeSessionId: 'sess-1',
  cols: 100,
  rows: 40,
  supervisor: false,
  mcpFacade: true,
  toolScope: 'operator',
  pluginTools: ['jira'],
  label: 'ship the thing',
  parentSessionId: 'parent-1',
  mcpItemIds: ['mcp-1'],
  targetHub: 'laptop',
};

describe('managedOptionsFromRequest', () => {
  it('carries the Fleet Manager role flags to a codex spawn', () => {
    const opts = managedOptionsFromRequest('codex', {
      cwd: '/home/u/Work',
      manager: true,
      fleetFullAccess: true,
      toolScope: 'operator',
      transport: 'stream',
    });
    // Without these two a codex manager is decorative: no isSupervisor (so
    // nudgeParentOnFinish refuses to wake it) and no grants on its token.
    expect(opts.manager).toBe(true);
    expect(opts.fleetFullAccess).toBe(true);
    expect(opts.toolScope).toBe('operator');
    expect(opts.transport).toBe('stream');
    expect(opts.provider).toBe('codex');
  });

  it('keeps transport off a provider whose adapter has no headless mode, loudly', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const opts = managedOptionsFromRequest('opencode', { transport: 'stream' });
    // The daemon adapter would reject the key, so it must not ride the payload
    // — but the request is reported rather than silently vanishing.
    expect(opts).not.toHaveProperty('transport');
    expect(warn.mock.calls.join(' ')).toMatch(/transport 'stream'/);
    warn.mockRestore();
  });

  it('folds every bypass spelling into skipPermissions', () => {
    // Managed providers only have ask/yolo, so the Claude spelling has to map
    // too — matching only 'yolo' left a bypassPermissions request unapplied.
    expect(managedOptionsFromRequest('codex', { permissionMode: 'yolo' }).skipPermissions).toBe(
      true,
    );
    expect(
      managedOptionsFromRequest('codex', { permissionMode: 'bypassPermissions' }).skipPermissions,
    ).toBe(true);
    expect(managedOptionsFromRequest('codex', { permissionMode: 'plan' }).skipPermissions).toBe(
      false,
    );
    expect(managedOptionsFromRequest('codex', { skipPermissions: true }).skipPermissions).toBe(
      true,
    );
  });

  it('is TOTAL over the request: no field is silently lost', () => {
    const opts = managedOptionsFromRequest('codex', FULL_REQUEST) as Record<string, unknown>;
    const unclassified: string[] = [];
    for (const [field, rule] of Object.entries(SPAWN_REQUEST_FIELDS)) {
      if (rule.kind === 'forward') {
        // 'forward' has to mean it actually arrives — with the request's value
        // where the option keeps the same name.
        if (!(field in opts) || opts[field] === undefined) unclassified.push(field);
      }
      // 'derived' / 'unsupported' are deliberate: the reason string is the
      // contract, and explainUnsupportedManagedOptions surfaces it at spawn.
      if (rule.kind === 'unsupported') expect(rule.why.length).toBeGreaterThan(10);
      if (rule.kind === 'derived') expect(rule.into.length).toBeGreaterThan(0);
    }
    expect(unclassified).toEqual([]);
  });
});

describe('explainUnsupportedManagedOptions', () => {
  it('says nothing when the provider can carry everything asked for', () => {
    expect(
      explainUnsupportedManagedOptions({
        provider: 'codex',
        transport: 'stream',
        manager: true,
        toolScope: 'operator',
        skipPermissions: true,
        permissionMode: 'yolo',
      }),
    ).toEqual([]);
    // Claude stream carries the full option set — never warn for it.
    expect(
      explainUnsupportedManagedOptions({
        provider: 'claude',
        transport: 'stream',
        profileId: 'work',
        mcpItemIds: ['a'],
        permissionMode: 'plan',
      }),
    ).toEqual([]);
  });

  it('names each option it cannot honour, with a reason', () => {
    const why = explainUnsupportedManagedOptions({
      provider: 'codex',
      profileId: 'work',
      mcpItemIds: ['a'],
      permissionMode: 'plan',
    });
    expect(why.join('\n')).toMatch(/profileId/);
    expect(why.join('\n')).toMatch(/mcpItemIds/);
    expect(why.join('\n')).toMatch(/permissionMode 'plan'/);
  });

  it('flags a facade asked of pi, which has no MCP client', () => {
    expect(
      explainUnsupportedManagedOptions({ provider: 'pi', toolScope: 'operator' }).join('\n'),
    ).toMatch(/facade/);
  });

  it("says copilot's facade MAY not attach, and that 'ask' is not an approval gate", () => {
    // Copilot's cliff is the only DYNAMIC one: the CLI has a real MCP client
    // (servers ride in on a flag), but a GitHub org policy can disable
    // third-party servers, and the CLI then reports zero of them and carries on
    // working. So the spawn cannot promise either way — it warns, and the
    // adapter raises a session error at runtime if they really didn't attach.
    const facade = explainUnsupportedManagedOptions({
      provider: 'copilot',
      toolScope: 'operator',
      skipPermissions: true,
    }).join('\n');
    expect(facade).toMatch(/facade/);
    expect(facade).toMatch(/org policy/);
    // Not the pi sentence: copilot is not "ships no MCP client".
    expect(facade).not.toMatch(/ships no MCP client/);

    // And the permission half: in `-p` mode Copilot cannot ask, so tools run
    // automatically and 'ask' means path confinement. Announced once at spawn
    // rather than left for the pill to imply.
    const asked = explainUnsupportedManagedOptions({ provider: 'copilot' }).join('\n');
    expect(asked).toMatch(/cannot ask/);
    // A yolo spawn is not lied to about approvals it never asked for.
    expect(
      explainUnsupportedManagedOptions({ provider: 'copilot', skipPermissions: true }).join('\n'),
    ).not.toMatch(/cannot ask/);
  });
});
