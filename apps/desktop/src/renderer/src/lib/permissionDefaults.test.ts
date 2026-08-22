import { describe, it, expect } from 'vitest';
import { currentPermissionModeDefault, permissionModeDefaultPatch } from './permissionDefaults';

// The two config keys are one decision. The Settings picker is the only writer,
// so these two helpers are the whole contract: what the file currently means,
// and what a click writes back.

describe('currentPermissionModeDefault', () => {
  it('absent config = approvals on', () => {
    expect(currentPermissionModeDefault(undefined)).toBe('');
    expect(currentPermissionModeDefault({})).toBe('');
  });

  it('reads a plain mode through', () => {
    expect(currentPermissionModeDefault({ defaultPermissionMode: 'plan' })).toBe('plan');
    expect(currentPermissionModeDefault({ defaultPermissionMode: 'acceptEdits' })).toBe(
      'acceptEdits',
    );
  });

  it('the skip flag wins — it is what the spawn dialog honours last', () => {
    expect(
      currentPermissionModeDefault({ defaultPermissionMode: 'plan', skipPermissionsDefault: true }),
    ).toBe('bypassPermissions');
  });

  it('an unrecognized mode fails CLOSED, not to bypass', () => {
    expect(currentPermissionModeDefault({ defaultPermissionMode: 'YOLO!!' })).toBe('');
    expect(currentPermissionModeDefault({ defaultPermissionMode: 'yolo' })).toBe('');
  });
});

describe('permissionModeDefaultPatch', () => {
  it('full access sets BOTH keys', () => {
    expect(permissionModeDefaultPatch('bypassPermissions')).toEqual({
      defaultPermissionMode: 'bypassPermissions',
      skipPermissionsDefault: true,
    });
  });

  it('any other mode CLEARS the skip flag, so the two can never contradict', () => {
    for (const mode of ['', 'plan', 'acceptEdits']) {
      expect(permissionModeDefaultPatch(mode)).toEqual({
        defaultPermissionMode: mode,
        skipPermissionsDefault: false,
      });
    }
  });

  it('round-trips: writing a mode and reading it back gives the same mode', () => {
    for (const mode of ['', 'plan', 'acceptEdits', 'bypassPermissions']) {
      expect(currentPermissionModeDefault(permissionModeDefaultPatch(mode))).toBe(mode);
    }
  });
});
