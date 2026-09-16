import { describe, expect, it } from 'vitest';
import { snapshotIsLocalLiveSession } from './snapshotLiveness';

describe('local session liveness', () => {
  const cases: Array<{ name: string; snapshot: unknown; live: boolean }> = [
    { name: 'active status', snapshot: { cwd: '/w/p', status: 'active' }, live: true },
    { name: 'no state yet', snapshot: { cwd: '/w/p' }, live: true },
    { name: 'ended status', snapshot: { cwd: '/w/p', status: 'ended' }, live: false },
    { name: 'stopped mode', snapshot: { cwd: '/w/p', mode: 'stopped' }, live: false },
    { name: 'archived', snapshot: { cwd: '/w/p', mode: 'running', archived: true }, live: false },
    { name: 'terminal mode', snapshot: { cwd: '/', mode: 'unknown' }, live: false },
    {
      name: 'remote live session',
      snapshot: { cwd: '/peer/p', status: 'active', hub: 'peer' },
      live: false,
    },
    { name: 'invalid row', snapshot: 'broken', live: false },
  ];

  for (const testCase of cases) {
    it(testCase.name, () => {
      expect(snapshotIsLocalLiveSession(testCase.snapshot)).toBe(testCase.live);
    });
  }
});
