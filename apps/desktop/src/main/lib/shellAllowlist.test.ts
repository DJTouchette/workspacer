import fs from 'fs';
import os from 'os';
import path from 'path';
import { afterEach, describe, expect, it } from 'vitest';

import { defaultShell, resolveTerminalShell, shellConfig } from './shellAllowlist';

// terminals.create's `shell` is argv[0] of a host process, straight from a bus
// caller. Nothing checked it in either provider, and capspec's own record for
// this capability named only `cwd`. Twin of the brain's
// TestTerminalShellIsAnAllowlistNotAPassthrough.
describe('terminals.create shell allowlist', () => {
  const realEtcShells = shellConfig.etcShellsPath;
  const realShell = process.env.SHELL;
  afterEach(() => {
    shellConfig.etcShellsPath = realEtcShells;
    if (realShell === undefined) delete process.env.SHELL;
    else process.env.SHELL = realShell;
  });

  const sandbox = (): string => fs.mkdtempSync(path.join(os.tmpdir(), 'wks-shells-'));

  it('allows the host login shells and nothing else', () => {
    const dir = sandbox();
    shellConfig.etcShellsPath = path.join(dir, 'shells');
    fs.writeFileSync(shellConfig.etcShellsPath, '# comment\n/bin/bash\n/usr/local/bin/xonsh\n\n');
    process.env.SHELL = '/bin/zsh';

    // The floor: each of the three ways a shell qualifies.
    for (const ok of ['/bin/zsh', '/bin/sh', '/usr/local/bin/xonsh']) {
      expect(resolveTerminalShell(ok)).toBe(ok);
    }
    expect(resolveTerminalShell(undefined)).toBe('/bin/zsh');
    expect(resolveTerminalShell('')).toBe('/bin/zsh');
    expect(defaultShell()).toBe('/bin/zsh');

    // The point: an arbitrary executable — including one the same caller could
    // have written into its own agent cwd with fs.write, which preserves the
    // 0755 mode of an existing file — is REFUSED, not silently downgraded.
    const planted = path.join(dir, 'node_modules', '.bin', 'tsc');
    fs.mkdirSync(path.dirname(planted), { recursive: true });
    fs.writeFileSync(planted, '#!/bin/sh\nid > /tmp/pwned\n', { mode: 0o755 });
    for (const bad of [planted, '/usr/bin/env', '/bin/sh -c id', '../../bin/sh', 'sh']) {
      expect(resolveTerminalShell(bad)).toBeNull();
    }
    // A comment line in /etc/shells is not a shell.
    expect(resolveTerminalShell('# comment')).toBeNull();
  });

  it('survives a missing /etc/shells on the platform fallbacks alone', () => {
    shellConfig.etcShellsPath = path.join(sandbox(), 'nope');
    process.env.SHELL = '/bin/zsh';
    expect(resolveTerminalShell('/bin/sh')).toBe('/bin/sh');
    expect(resolveTerminalShell('/usr/local/bin/xonsh')).toBeNull();
  });
});
