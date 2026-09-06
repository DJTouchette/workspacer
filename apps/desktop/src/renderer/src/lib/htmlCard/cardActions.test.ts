import { it, expect, beforeEach, afterEach, vi } from 'vitest';
import { performCardAction } from './cardActions';
import { SESSION_WATCH_EVENT } from '../watchBus';
import { LIBRARY_INSERT_EVENT } from '../libraryBus';
const HOST = { sessionId: 'owner', paneId: 'own-pane', cwd: '/project' };
const owner = { sessionId: 'owner', status: 'active', cwd: '/project' };
const child = { sessionId: 'worker', parentSessionId: 'owner', status: 'active', cwd: '/worktree' };
const original = window.electronAPI;
const get = vi.fn();
const read = vi.fn();
beforeEach(() => {
  get.mockImplementation(async (id) => (id === 'owner' ? owner : child));
  read.mockResolvedValue({ ok: true, path: '/project/a', before: 'old', after: 'new' });
  window.electronAPI = { getClaudeSession: get, htmlCardReadDiff: read } as never;
});
afterEach(() => {
  window.electronAPI = original;
  vi.restoreAllMocks();
  vi.clearAllMocks();
});
it('opens only live direct children, never foreign, ended or gone targets', async () => {
  const events = vi.spyOn(window, 'dispatchEvent');
  const action = { kind: 'open_worker', label: 'Open', sessionId: 'worker' } as const;
  expect((await performCardAction(action, HOST)).ok).toBe(true);
  expect(events.mock.calls.map(([e]) => e.type)).toContain(SESSION_WATCH_EVENT);
  for (const target of [
    null,
    { ...child, status: 'ended' },
    { ...child, parentSessionId: 'foreign' },
    { ...child, hub: 'remote' },
  ]) {
    events.mockClear();
    get.mockImplementation(async (id) => (id === 'owner' ? owner : target));
    expect((await performCardAction(action, HOST)).ok).toBe(false);
    expect(events.mock.calls.map(([e]) => e.type)).not.toContain(SESSION_WATCH_EVENT);
  }
});
it('uses the owner id for backend diff reads and presents returned bytes without a review bus', async () => {
  const show = vi.fn();
  const events = vi.spyOn(window, 'dispatchEvent');
  expect(
    (await performCardAction({ kind: 'view_diff', label: 'Diff', path: 'a' }, HOST, show)).ok,
  ).toBe(true);
  expect(read).toHaveBeenCalledWith('a', 'owner');
  expect(show).toHaveBeenCalledWith({ ok: true, path: '/project/a', before: 'old', after: 'new' });
  expect(events).not.toHaveBeenCalled();
});
it('fails gracefully on old/web clients missing the guarded diff method', async () => {
  window.electronAPI = { getClaudeSession: get } as never;
  expect(
    (
      await performCardAction(
        { kind: 'view_diff', label: 'Diff', path: '../escape' },
        HOST,
        vi.fn(),
      )
    ).ok,
  ).toBe(false);
});
it('prefills only the owning session AND pane and never sends', async () => {
  const events = vi.spyOn(window, 'dispatchEvent');
  expect(
    (await performCardAction({ kind: 'fill_composer', label: 'Draft', text: 'hello' }, HOST)).ok,
  ).toBe(true);
  expect(events.mock.calls.map(([e]) => e.type)).toEqual([LIBRARY_INSERT_EVENT]);
  expect((events.mock.calls[0][0] as CustomEvent).detail).toEqual({
    sessionId: 'owner',
    paneId: 'own-pane',
    text: 'hello',
  });
});
it('refuses stale owner identities including after async target resolution', async () => {
  const action = { kind: 'fill_composer', label: 'Draft', text: 'x' } as const;
  for (const snapshot of [null, { ...owner, status: 'ended' }, { ...owner, hub: 'remote' }]) {
    get.mockResolvedValue(snapshot);
    expect((await performCardAction(action, HOST)).ok).toBe(false);
  }
  get.mockResolvedValue(owner);
  expect((await performCardAction(action, { ...HOST, isCurrent: () => false })).ok).toBe(false);
  const alive = vi.fn().mockReturnValueOnce(true).mockReturnValue(false);
  expect((await performCardAction(action, { ...HOST, isCurrent: alive })).ok).toBe(false);
});
