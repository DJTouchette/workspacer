/**
 * The roots are the whole width of the file: allowance, so what actually
 * REACHES them from config is the thing worth pinning. `config.projects` is read
 * here and nowhere else for this purpose; a projection that quietly returned
 * only `[home]` would look identical in every guard test above and would refuse
 * every project that lives outside the home tree.
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';
import * as os from 'os';

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
});

describe('webviewFileRoots', () => {
  it('always includes the home directory', () => {
    getConfig.mockReturnValue({ projects: {} });
    expect(webviewFileRoots()).toEqual([os.homedir()]);
  });

  it('includes every configured project directory, home or not', () => {
    getConfig.mockReturnValue({
      projects: {
        '/srv/mounted/repo': { name: 'repo' },
        'D:\\work\\thing': { name: 'thing' },
      },
    });
    const roots = webviewFileRoots();
    expect(roots).toContain(os.homedir());
    expect(roots).toContain('/srv/mounted/repo');
    expect(roots).toContain('D:\\work\\thing');
  });

  it('drops blank keys rather than letting one become a wildcard', () => {
    getConfig.mockReturnValue({ projects: { '': { name: 'x' }, '   ': { name: 'y' } } });
    expect(webviewFileRoots()).toEqual([os.homedir()]);
  });

  it('tolerates a config with no projects map at all', () => {
    getConfig.mockReturnValue({});
    expect(webviewFileRoots()).toEqual([os.homedir()]);
  });

  it('narrows to home — never widens, never throws — when the config cannot be read', () => {
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
