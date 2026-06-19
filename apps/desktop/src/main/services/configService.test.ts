import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';

// ─── isolate module per test ────────────────────────────────────────────────
// We need to control process.platform and process.env before the module is
// imported, so all tests use vi.isolateModules() where platform matters, or
// we pull the exported helper directly.

// ─── deepMerge characterization ─────────────────────────────────────────────
// deepMerge is private, but its semantics are observable through getConfigDir
// (a thin wrapper) and saveConfig / reloadConfig.  The cleanest surface is to
// test the *exported* getConfigDir plus the module-level side-effects.
// For deepMerge we import the module with fs mocked so the ConfigService
// constructor succeeds without touching disk.

describe('getConfigDir – platform branches', () => {
  const realPlatform = process.platform;
  const realEnv = { ...process.env };

  afterEach(() => {
    // Restore platform + env
    Object.defineProperty(process, 'platform', { value: realPlatform, configurable: true });
    process.env = { ...realEnv };
    vi.resetModules();
  });

  it('uses XDG_CONFIG_HOME when set (non-win32)', async () => {
    Object.defineProperty(process, 'platform', { value: 'linux', configurable: true });
    process.env.XDG_CONFIG_HOME = '/custom/xdg';
    delete process.env.APPDATA;

    const { getConfigDir } = await import('./configService');
    expect(getConfigDir()).toBe('/custom/xdg/workspacer');
  });

  it('falls back to homedir/.config/workspacer when XDG_CONFIG_HOME is absent (linux)', async () => {
    Object.defineProperty(process, 'platform', { value: 'linux', configurable: true });
    delete process.env.XDG_CONFIG_HOME;
    delete process.env.APPDATA;

    const os = await import('os');
    const home = os.homedir();

    const { getConfigDir } = await import('./configService');
    expect(getConfigDir()).toBe(`${home}/.config/workspacer`);
  });

  it('falls back to homedir/.config/workspacer on darwin too', async () => {
    Object.defineProperty(process, 'platform', { value: 'darwin', configurable: true });
    delete process.env.XDG_CONFIG_HOME;
    delete process.env.APPDATA;

    const os = await import('os');
    const home = os.homedir();

    const { getConfigDir } = await import('./configService');
    expect(getConfigDir()).toBe(`${home}/.config/workspacer`);
  });

  it('uses APPDATA on win32 when set', async () => {
    Object.defineProperty(process, 'platform', { value: 'win32', configurable: true });
    process.env.APPDATA = 'C:\\Users\\user\\AppData\\Roaming';

    const path = await import('path');
    const { getConfigDir } = await import('./configService');
    // path.join is the host OS join (linux uses /), so we match whatever it
    // actually produces rather than hard-coding a Windows separator.
    expect(getConfigDir()).toBe(path.join('C:\\Users\\user\\AppData\\Roaming', 'workspacer'));
  });

  it('falls back to homedir/AppData/Roaming/workspacer on win32 when APPDATA is absent', async () => {
    Object.defineProperty(process, 'platform', { value: 'win32', configurable: true });
    delete process.env.APPDATA;

    const os = await import('os');
    const home = os.homedir();
    const path = await import('path');

    const { getConfigDir } = await import('./configService');
    expect(getConfigDir()).toBe(path.join(home, 'AppData', 'Roaming', 'workspacer'));
  });
});

// ─── deepMerge semantics ─────────────────────────────────────────────────────
// We test deepMerge indirectly through configService.saveConfig / getConfig.
// We mock fs so the constructor does not read or write real files.

vi.mock('fs', () => ({
  readFileSync: vi.fn().mockImplementation(() => {
    throw new Error('ENOENT');
  }),
  writeFileSync: vi.fn(),
  mkdirSync: vi.fn(),
}));

// Import after the mock is registered so the ConfigService constructor sees it.
// Because vitest hoists vi.mock, this import runs after the mock.
import { configService } from './configService';

describe('deepMerge semantics – via configService.saveConfig', () => {
  beforeEach(() => {
    // Reset to defaults between tests by reloading from the (mocked) disk
    // which always throws ENOENT, so loadFromDisk returns pure defaults.
    // We cannot call reloadConfig() because it also tries writeDefaults(), but
    // that's mocked out so it's safe.
    configService.reloadConfig();
  });

  it('merges nested object keys without clobbering sibling keys', () => {
    // Save only ui.theme; ui.animations and other siblings must survive.
    configService.saveConfig({ ui: { theme: 'light' } as any });
    const cfg = configService.getConfig();

    expect(cfg.ui.theme).toBe('light');
    // Default animations is false – it must still be there
    expect(cfg.ui.animations).toBe(false);
    expect(cfg.ui.fontSize).toBe(14);
  });

  it('replaces arrays wholesale (array-replace semantics)', () => {
    const onlyBash = [{ name: 'bash', path: '/bin/bash', label: 'Bash' }];
    configService.saveConfig({ terminal: { shells: onlyBash } as any });
    const cfg = configService.getConfig();

    // The whole array must be the new one, not merged per-element
    expect(cfg.terminal.shells).toEqual(onlyBash);
    expect(cfg.terminal.shells).toHaveLength(1);
  });

  it('preserves unrelated top-level sections when saving a partial', () => {
    configService.saveConfig({ session: { autoResume: false } });
    const cfg = configService.getConfig();

    // session changed
    expect(cfg.session.autoResume).toBe(false);
    // browser defaults untouched
    expect(cfg.browser.homepage).toBe('https://google.com');
    expect(cfg.browser.hibernateAfter).toBe(300);
  });

  it('deeply merges multiple layers of nesting', () => {
    // keybindings.shortcuts is a plain Record (object, not array)
    configService.saveConfig({
      keybindings: {
        shortcuts: { 'new-terminal': 'ctrl+shift+t' },
      } as any,
    });
    const cfg = configService.getConfig();

    // The changed shortcut
    expect(cfg.keybindings.shortcuts['new-terminal']).toBe('ctrl+shift+t');
    // Other shortcuts survive (grouped prefix chord default)
    expect(cfg.keybindings.shortcuts['close-pane']).toBe('prefix t w');
    // Sibling keys of shortcuts survive
    expect(cfg.keybindings.prefix).toBe('ctrl+space');
  });

  it('replaces a key with a falsy value (false) rather than keeping the default', () => {
    configService.saveConfig({ notifications: { sound: true } as any });
    expect(configService.getConfig().notifications.sound).toBe(true);

    configService.saveConfig({ notifications: { sound: false } as any });
    expect(configService.getConfig().notifications.sound).toBe(false);
  });

  it('overwrites a string value with an empty string', () => {
    configService.saveConfig({ ui: { theme: '' } as any });
    expect(configService.getConfig().ui.theme).toBe('');
  });

  it('replaces panes.default array entirely', () => {
    const newDefault = [{ id: 'x', type: 'terminal', title: 'X', width: 600, order: 0 }];
    configService.saveConfig({ panes: { default: newDefault } as any });
    const cfg = configService.getConfig();

    expect(cfg.panes.default).toEqual(newDefault);
    // Other panes fields survive
    expect(cfg.panes.gap).toBe(16);
    expect(cfg.panes.viewMode).toBe('tabs');
  });

  it('saves a new scripts entry without touching other top-level keys', () => {
    const scripts = { '/home/user/proj': [{ name: 'build', command: 'make' }] };
    configService.saveConfig({ scripts });
    const cfg = configService.getConfig();

    expect(cfg.scripts).toEqual(scripts);
    // terminal defaults survive
    expect(cfg.terminal.fontSize).toBe(14);
  });

  it('source null/undefined values do not overwrite target (deepMerge guard)', () => {
    // deepMerge checks `if (!source || typeof source !== 'object') return target`
    // Passing an explicit null for a leaf via a cast
    configService.saveConfig({ ui: { theme: null } as any });
    const cfg = configService.getConfig();
    // null is not an object so the else-branch fires: result[key] = null
    // This characterizes the ACTUAL behavior (null replaces the value)
    expect(cfg.ui.theme).toBeNull();
  });

  it('deepMerge with null source returns target unchanged', () => {
    // saveConfig calls deepMerge(this.config, partial). If partial is null-ish
    // the top guard `if (!source ...) return target` fires.
    // saveConfig passes partial directly — passing null/undefined would be a
    // type error at the call site, so characterise the safe path: empty object.
    const before = configService.getConfig().ui.theme;
    configService.saveConfig({});
    expect(configService.getConfig().ui.theme).toBe(before);
  });
});
