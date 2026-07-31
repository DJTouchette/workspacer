import { describe, it, expect } from 'vitest';
import { scrubBypassArgs, scrubBypassProfile } from './claudeProfiles';

/**
 * The remote spawn capability forces `skipPermissions` off and drops a bypass
 * `permissionMode` — but a Claude PROFILE carries its own `extraArgs`, and a bus
 * caller can both create profiles and pick one by id. Clamping the request's
 * fields while passing `profileId` through untouched let the same bypass in
 * through the side door, on the desktop path only (the Go brain already
 * scrubbed). These cases mirror scrubBypassArgs in
 * services/hub/cmd/brain/profiles.go — keep the two in step.
 */

describe('scrubBypassArgs', () => {
  it('drops the skip-permissions flag', () => {
    expect(scrubBypassArgs(['--dangerously-skip-permissions'])).toEqual([]);
    expect(scrubBypassArgs(['--model', 'opus', '--dangerously-skip-permissions'])).toEqual([
      '--model',
      'opus',
    ]);
  });

  it('drops a bypass permission-mode in the space form, value included', () => {
    expect(scrubBypassArgs(['--permission-mode', 'bypassPermissions'])).toEqual([]);
    expect(scrubBypassArgs(['--permission-mode', 'yolo', '--model', 'opus'])).toEqual([
      '--model',
      'opus',
    ]);
  });

  it('drops a bypass permission-mode in the equals form', () => {
    expect(scrubBypassArgs(['--permission-mode=bypassPermissions'])).toEqual([]);
    expect(scrubBypassArgs(['--permission-mode=yolo'])).toEqual([]);
  });

  it('leaves every non-bypass mode alone — this is a clamp, not a ban', () => {
    expect(scrubBypassArgs(['--permission-mode', 'acceptEdits'])).toEqual([
      '--permission-mode',
      'acceptEdits',
    ]);
    expect(scrubBypassArgs(['--permission-mode=plan'])).toEqual(['--permission-mode=plan']);
    expect(scrubBypassArgs(['--permission-mode', 'default'])).toEqual([
      '--permission-mode',
      'default',
    ]);
  });

  it('keeps unrelated args, including a trailing lone --permission-mode', () => {
    expect(scrubBypassArgs(['--verbose', '--settings', '/tmp/s.json'])).toEqual([
      '--verbose',
      '--settings',
      '/tmp/s.json',
    ]);
    // No value to inspect: keep it and let the CLI complain, rather than
    // swallowing an arg we don't understand.
    expect(scrubBypassArgs(['--permission-mode'])).toEqual(['--permission-mode']);
  });

  it('survives empty and absent input', () => {
    expect(scrubBypassArgs([])).toEqual([]);
    expect(scrubBypassArgs(undefined)).toEqual([]);
  });
});

describe('scrubBypassProfile', () => {
  it('returns a copy with clean args and everything else intact', () => {
    const profile = {
      id: 'p1',
      name: 'YOLO',
      configDir: '~/.claude-yolo',
      extraArgs: ['--dangerously-skip-permissions', '--verbose'],
      isDefault: false,
    };
    const clean = scrubBypassProfile(profile)!;
    expect(clean.extraArgs).toEqual(['--verbose']);
    expect(clean.configDir).toBe('~/.claude-yolo');
    expect(clean.name).toBe('YOLO');
    // The stored profile is untouched — this is a per-spawn view of it, not an
    // edit of the user's own profile.
    expect(profile.extraArgs).toEqual(['--dangerously-skip-permissions', '--verbose']);
  });

  it('passes undefined through (no profile chosen)', () => {
    expect(scrubBypassProfile(undefined)).toBeUndefined();
  });
});
