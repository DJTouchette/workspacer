/**
 * installWorkspacerCli spawns the bundled `workspacer install-cli` and reports
 * what it printed. These tests point the dev-mode resolution at a temp repo
 * layout (<tmp>/services/hub/workspacer) holding a stub script, so we exercise
 * the real spawn + output capture without the actual Go binary.
 *
 * Strategy (mirrors claudeResolver.test.ts): mock 'electron' before importing
 * the module under test — app.getAppPath() points two levels below the temp
 * "repo" root, exactly like apps/desktop under the real one.
 */

import { describe, it, expect, vi, beforeAll, afterAll } from 'vitest';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-cli-install-'));
const appPath = path.join(tmpRoot, 'apps', 'desktop'); // <repo>/apps/desktop
const binDir = path.join(tmpRoot, 'services', 'hub');
const binPath = path.join(binDir, 'workspacer');

vi.mock('electron', () => ({
  app: {
    isPackaged: false,
    getAppPath: () => appPath,
  },
}));

// Notices are pushed to the (absent) main window — stub so nothing throws and
// we can assert the level mirrors the result.
const notifySystem = vi.hoisted(() => vi.fn());
vi.mock('./systemNotice', () => ({ notifySystem }));

const { installWorkspacerCli, workspacerCliPath } = await import('./cliInstall');

/** (Re)write the stub CLI script with the given body. */
function writeStub(body: string): void {
  fs.mkdirSync(binDir, { recursive: true });
  fs.writeFileSync(binPath, `#!/bin/sh\n${body}\n`, { mode: 0o755 });
}

beforeAll(() => {
  fs.mkdirSync(appPath, { recursive: true });
});

afterAll(() => {
  fs.rmSync(tmpRoot, { recursive: true, force: true });
});

describe('workspacerCliPath', () => {
  it('resolves the dev-mode sibling-repo path', () => {
    expect(workspacerCliPath()).toBe(binPath);
  });
});

describe('installWorkspacerCli', () => {
  it('fails with a pointer at the expected path when the binary is missing', async () => {
    fs.rmSync(binPath, { force: true });
    const res = await installWorkspacerCli();
    expect(res.ok).toBe(false);
    expect(res.message).toContain(binPath);
    expect(notifySystem).toHaveBeenLastCalledWith(expect.objectContaining({ level: 'error' }));
  });

  it('returns ok + the CLI output (install line and PATH instructions) on exit 0', async () => {
    writeStub(
      'echo "installed /home/u/.local/bin/workspacer -> /opt/workspacer"\necho "/home/u/.local/bin is not on your PATH. Add it:"',
    );
    const res = await installWorkspacerCli();
    expect(res.ok).toBe(true);
    expect(res.message).toContain('installed /home/u/.local/bin/workspacer');
    expect(res.message).toContain('not on your PATH');
    expect(notifySystem).toHaveBeenLastCalledWith(expect.objectContaining({ level: 'info' }));
  });

  it('returns ok:false with the CLI stderr on a non-zero exit', async () => {
    writeStub('echo "workspacer: install to /usr/local/bin failed" >&2\nexit 1');
    const res = await installWorkspacerCli();
    expect(res.ok).toBe(false);
    expect(res.message).toContain('install to /usr/local/bin failed');
    expect(notifySystem).toHaveBeenLastCalledWith(expect.objectContaining({ level: 'error' }));
  });
});
