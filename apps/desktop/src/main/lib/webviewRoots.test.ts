/**
 * The roots are the whole width of the file: allowance, so what actually
 * REACHES them from config is the thing worth pinning. `config.projects` is read
 * here and nowhere else for this purpose; a projection that quietly returned
 * only `[home]` would look identical in every guard test above and would refuse
 * every project that lives outside the home tree.
 */

import { describe, it, expect, vi, beforeEach, beforeAll, afterAll } from 'vitest';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

const getConfig = vi.fn();
vi.mock('../services/configService', () => ({
  configService: {
    getConfig: () => getConfig(),
  },
  getConfigDir: () => '/tmp/wks-cfg',
}));

import { webviewFileRoots } from './webviewRoots';

beforeEach(() => {
  getConfig.mockReset();
  vi.spyOn(console, 'warn').mockImplementation(() => {});
});

describe('webviewFileRoots', () => {
  it('always includes the home directory', () => {
    getConfig.mockReturnValue({ projects: {} });
    expect(webviewFileRoots()).toEqual([os.homedir()]);
  });

  it('includes every configured project directory, home or not', () => {
    // The drive spelling is absolute only ON Windows, and a root that is not
    // absolute for THIS platform is dropped (see the second describe below), so
    // it is asserted where it means something and not where it does not.
    const winProject = 'D:\\work\\thing';
    getConfig.mockReturnValue({
      projects: {
        '/srv/mounted/repo': { name: 'repo' },
        [winProject]: { name: 'thing' },
      },
    });
    const roots = webviewFileRoots();
    expect(roots).toContain(os.homedir());
    expect(roots).toContain('/srv/mounted/repo');
    expect(roots.includes(winProject)).toBe(process.platform === 'win32');
  });

  it('drops blank keys rather than letting one become a wildcard', () => {
    getConfig.mockReturnValue({ projects: { '': { name: 'x' }, '   ': { name: 'y' } } });
    expect(webviewFileRoots()).toEqual([os.homedir()]);
  });

  it('tolerates a config with no projects map at all', () => {
    getConfig.mockReturnValue({});
    expect(webviewFileRoots()).toEqual([os.homedir()]);
  });

  it('narrows to home, never widens and never throws, when the config cannot be read', () => {
    getConfig.mockImplementation(() => {
      throw new Error('config.yaml is a directory');
    });
    expect(webviewFileRoots()).toEqual([os.homedir()]);
  });

  it('does not repeat a project that is already the home directory', () => {
    getConfig.mockReturnValue({ projects: { [os.homedir()]: { name: 'home' } } });
    expect(webviewFileRoots()).toEqual([os.homedir()]);
  });
});

/**
 * A root that resolves to the filesystem root is not a narrow root, it is the
 * absence of one: every readable file on the host becomes a page the browser
 * pane will render. `config.projects` is written by the desktop AND the Go
 * brain, and `projects.add` rides the bus, so its keys are not all typed by a
 * person at a dialog. Same rule, and the same reasoning, as the volume-root
 * check plugin/manager.go applies to a plugin's declared path scopes.
 */
describe('webviewFileRoots: a root that is not a scope', () => {
  let tmp: string;

  beforeAll(() => {
    tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-webviewroots-'));
    fs.mkdirSync(path.join(tmp, 'real'), { recursive: true });
    // The shape the textual test cannot see: an ordinary-looking project
    // directory that IS the filesystem root once its symlink is read.
    fs.symlinkSync('/', path.join(tmp, 'everything'));
  });

  afterAll(() => {
    fs.rmSync(tmp, { recursive: true, force: true });
  });

  const rootsFor = (...dirs: string[]) => {
    getConfig.mockReturnValue({ projects: Object.fromEntries(dirs.map((d) => [d, { name: d }])) });
    return webviewFileRoots();
  };

  it("refuses '/' outright", () => {
    expect(rootsFor('/')).toEqual([os.homedir()]);
  });

  it("refuses '//', which canonicalizes to the same place", () => {
    expect(rootsFor('//')).toEqual([os.homedir()]);
  });

  it('refuses a relative path, which can confine nothing', () => {
    expect(rootsFor('projects/site', './x', '~/work')).toEqual([os.homedir()]);
  });

  it('refuses a root that is a SYMLINK to the filesystem root', () => {
    expect(rootsFor(path.join(tmp, 'everything'))).toEqual([os.homedir()]);
  });

  it('still admits an ordinary project directory beside the refused ones', () => {
    const good = path.join(tmp, 'real');
    const roots = rootsFor('/', good, path.join(tmp, 'everything'));
    expect(roots).toContain(good);
    expect(roots).toHaveLength(2); // home + the good one
  });

  it('logs a refused root once, not once per check', () => {
    const warn = console.warn as unknown as ReturnType<typeof vi.fn>;
    warn.mockClear();
    rootsFor('/srv/relative/../..');
    rootsFor('/srv/relative/../..');
    rootsFor('/srv/relative/../..');
    expect(warn.mock.calls.filter((c) => String(c[0]).includes('/srv/relative'))).toHaveLength(1);
  });
});
