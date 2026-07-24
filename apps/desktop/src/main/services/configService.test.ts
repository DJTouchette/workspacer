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

const enoent = () => {
  const err = new Error('ENOENT: no such file or directory') as NodeJS.ErrnoException;
  err.code = 'ENOENT';
  throw err;
};

vi.mock('fs', () => ({
  readFileSync: vi.fn().mockImplementation(() => enoent()),
  writeFileSync: vi.fn(),
  mkdirSync: vi.fn(),
  copyFileSync: vi.fn(),
  // atomicWriteFileSync (temp-file + rename) backs every config write now, so the
  // rename/chmod/rm primitives must exist on the mock or the write path throws.
  renameSync: vi.fn(),
  chmodSync: vi.fn(),
  rmSync: vi.fn(),
  // Default: no file → mtime gate stays inert (configMtimeMs returns 0), so the
  // existing in-memory-cache tests behave exactly as before. The mtime-gate
  // suite below drives statSync explicitly.
  statSync: vi.fn().mockImplementation(() => enoent()),
}));

// Import after the mock is registered so the ConfigService constructor sees it.
// Because vitest hoists vi.mock, this import runs after the mock.
import * as fsMock from 'fs';
import { configService } from './configService';

const mockedFs = vi.mocked(fsMock);

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

  it('persists the spawn defaults (model + permission mode) without clobbering claude siblings', () => {
    // The exact partial the "new agent" flow saves (App.tsx handleSpawnAgent):
    // the picked model, the bypass toggle, and the permission mode. All three
    // must stick so the next new agent reopens on them instead of the defaults.
    configService.saveConfig({
      claude: {
        defaultModel: 'opus',
        skipPermissionsDefault: false,
        defaultPermissionMode: 'plan',
      } as any,
    });
    const cfg = configService.getConfig() as any;

    expect(cfg.claude.defaultModel).toBe('opus');
    expect(cfg.claude.defaultPermissionMode).toBe('plan');
    // Sibling claude defaults survive the partial save.
    expect(cfg.claude.defaultView).toBe('terminal');
    expect(cfg.claude.transport).toBe('stream');
  });

  it('preserves unrelated top-level sections when saving a partial', () => {
    configService.saveConfig({ supervisor: { pollSeconds: 99 } as any });
    const cfg = configService.getConfig();

    // supervisor changed
    expect(cfg.supervisor.pollSeconds).toBe(99);
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
    // Other shortcuts survive (flat prefix chord default)
    expect(cfg.keybindings.shortcuts['close-pane']).toBe('prefix w');
    // Sibling keys of shortcuts survive (default prefix from the VS Code preset)
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
    expect(cfg.panes.gap).toBe(0);
    expect(cfg.panes.tabPosition).toBe('top');
  });

  it('defaults ui.mode to fleet', () => {
    expect(configService.getConfig().ui.mode).toBe('fleet');
  });

  it('saves a new scripts entry without touching other top-level keys', () => {
    const scripts = { '/home/user/proj': [{ name: 'build', command: 'make' }] };
    configService.saveConfig({ scripts });
    const cfg = configService.getConfig();

    expect(cfg.scripts).toEqual(scripts);
    // terminal defaults survive
    expect(cfg.terminal.fontSize).toBe(14);
  });

  it('replaces ui.customThemes wholesale — deleting a custom theme persists', () => {
    const two = {
      'custom:one': { name: 'One', base: 'dark', colors: { accent: '#ff0000' } },
      'custom:two': { name: 'Two', base: 'nord', colors: { accent: '#00ff00' } },
    };
    configService.saveConfig({ ui: { customThemes: two } as any });
    expect(Object.keys(configService.getConfig().ui.customThemes ?? {})).toHaveLength(2);

    // Delete one theme: the saved map is the whole truth — deep-merge must NOT
    // resurrect the removed entry.
    const one = { 'custom:two': two['custom:two'] };
    configService.saveConfig({ ui: { customThemes: one } as any });
    const cfg = configService.getConfig();
    expect(cfg.ui.customThemes).toEqual(one);
    // Sibling ui keys survive the partial save.
    expect(cfg.ui.theme).toBe('everforest');
  });

  it('replaces claude.budgets wholesale — clearing a per-session budget persists', () => {
    // Arm a per-session budget, exactly like the inspector's "set budget" flow.
    configService.saveConfig({ claude: { budgets: { A: 5 } } as any });
    expect((configService.getConfig() as any).claude.budgets).toEqual({ A: 5 });

    // Clear it: the inspector sends claude.budgets with session A's key removed
    // (InspectorCard `delete budgets[sessionId]`). The saved map is the whole
    // truth — deep-merge must NOT resurrect the removed entry, or the budget can
    // never be cleared and budgetWatcher keeps enforcing the stale threshold.
    configService.saveConfig({ claude: { budgets: {} } as any });
    const cfg = configService.getConfig() as any;
    expect(cfg.claude.budgets).toEqual({});
    // Sibling claude defaults survive the wholesale budget replace.
    expect(cfg.claude.transport).toBe('stream');
  });

  it('source null/undefined values do not overwrite target (deepMerge guard)', () => {
    // A null leaf must NOT clobber the default — null means "unset", so the
    // default value survives.
    configService.saveConfig({ ui: { theme: null } as any });
    const cfg = configService.getConfig();
    expect(cfg.ui.theme).toBe('everforest');
  });

  it("an empty/null config section does not wipe that section's defaults", () => {
    // A bare `ui:` line in config.yaml parses to { ui: null }. deepMerge must
    // keep all ui defaults instead of replacing the section with null.
    configService.saveConfig({ ui: null } as any);
    const cfg = configService.getConfig();
    expect(cfg.ui).not.toBeNull();
    expect(cfg.ui.theme).toBe('everforest');
    expect(cfg.ui.animations).toBe(false);
    expect(cfg.ui.fontSize).toBe(14);
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

// ─── default-config single-source drift guard ────────────────────────────────
// The default config has ONE source of truth: services/hub/cmd/brain/
// config_defaults.json (the brain go:embeds it; the desktop consumes it through
// the generated configDefaults.generated.ts). If someone edits the JSON without
// re-running `npm run gen:config-defaults`, the committed generated module falls
// out of sync — this test catches that so the two runtimes can't drift.
import { CONFIG_DEFAULTS } from './configDefaults.generated';
import { CONFIG_DEFAULTS as RENDERER_CONFIG_DEFAULTS } from '../../renderer/src/hooks/configDefaults.generated';
import brainDefaults from '../../../../../services/hub/cmd/brain/config_defaults.json';

describe('default-config single source — generated TS matches the canonical brain JSON', () => {
  it('the main-process generated defaults deep-equal config_defaults.json', () => {
    expect(CONFIG_DEFAULTS).toEqual(brainDefaults);
  });

  it('the renderer generated defaults deep-equal config_defaults.json (no third drift copy)', () => {
    // Renderer + main build graphs don't share modules, so each has its own
    // generated leaf; both come from the one JSON via gen-config-defaults.mjs.
    expect(RENDERER_CONFIG_DEFAULTS).toEqual(brainDefaults);
  });

  it('carries the sections that used to be missing on the brain side', () => {
    // Regression guard for the historical drift: brain lacked agents/updates and
    // several claude fields entirely, so web/mobile fell back to different values.
    expect(brainDefaults).toHaveProperty('agents.binaries.claude');
    expect(brainDefaults).toHaveProperty('updates.channel');
    expect(brainDefaults).toHaveProperty('claude.transport', 'stream');
    expect(brainDefaults).toHaveProperty('ui.diffView');
    // editor.vim was removed outright (dead since the in-app CodeMirror editor
    // left); the editor block itself must still be present on the brain side.
    expect(brainDefaults).toHaveProperty('editor.terminalCommand');
    expect(brainDefaults).not.toHaveProperty('editor.vim');
    // supervisor.provider joined the canonical defaults (it was UI-written but
    // schema-absent before).
    expect(brainDefaults).toHaveProperty('supervisor.provider', 'claude');
  });
});

// ─── mtime gate — two writers (desktop + brain) on one config.yaml ───────────
// The desktop is no longer the only process writing config.yaml: the headless
// brain serves config.save over the hub bus for the web/mobile clients. Without
// an mtime gate, a main-process save here would deep-merge onto the startup
// cache and revert whatever the brain persisted after launch ("settings getting
// reset"). getConfig/saveConfig must re-read when the file changed underneath.

describe('mtime gate — folds in external (brain) writes instead of clobbering', () => {
  beforeEach(() => {
    mockedFs.readFileSync.mockReset();
    mockedFs.writeFileSync.mockReset();
    mockedFs.mkdirSync.mockReset();
    mockedFs.copyFileSync.mockReset();
    vi.mocked(fsMock.statSync).mockReset();
  });

  afterEach(() => {
    // Leave the singleton in the healthy first-run state for other suites.
    mockedFs.readFileSync.mockReset().mockImplementation(() => enoent());
    vi.mocked(fsMock.statSync)
      .mockReset()
      .mockImplementation(() => enoent());
    configService.reloadConfig();
  });

  it('getConfig re-reads when config.yaml changed under it (brain wrote a newer file)', () => {
    // Loaded state: theme dark at mtime 100.
    mockedFs.readFileSync.mockReturnValue('ui:\n  theme: dark\n');
    vi.mocked(fsMock.statSync).mockReturnValue({ mtimeMs: 100 } as any);
    configService.reloadConfig();
    expect(configService.getConfig().ui.theme).toBe('dark');

    // The brain rewrites config.yaml (theme nord) with a strictly newer mtime.
    mockedFs.readFileSync.mockReturnValue('ui:\n  theme: nord\n');
    vi.mocked(fsMock.statSync).mockReturnValue({ mtimeMs: 200 } as any);
    expect(configService.getConfig().ui.theme).toBe('nord');
  });

  it('does NOT re-read when the mtime is unchanged (steady state keeps the cache)', () => {
    mockedFs.readFileSync.mockReturnValue('ui:\n  theme: dark\n');
    vi.mocked(fsMock.statSync).mockReturnValue({ mtimeMs: 100 } as any);
    configService.reloadConfig();

    // A save applies in memory; the file "didn't change" (same mtime), so a
    // subsequent read must return the in-memory value, not re-parse stale disk.
    configService.saveConfig({ ui: { theme: 'light' } as any });
    mockedFs.readFileSync.mockReturnValue('ui:\n  theme: dark\n'); // disk still "dark"
    expect(configService.getConfig().ui.theme).toBe('light');
  });

  it('saveConfig folds in an external change instead of clobbering it', () => {
    // Desktop loaded theme dark at mtime 100.
    mockedFs.readFileSync.mockReturnValue('ui:\n  theme: dark\n');
    vi.mocked(fsMock.statSync).mockReturnValue({ mtimeMs: 100 } as any);
    configService.reloadConfig();

    // The brain changes the THEME (nord) at a newer mtime. The desktop then
    // saves an UNRELATED partial (seenModels, as usageAccumulator does) — it
    // must fold in the brain's theme, not revert it to the cached dark.
    mockedFs.readFileSync.mockReturnValue('ui:\n  theme: nord\n');
    vi.mocked(fsMock.statSync).mockReturnValue({ mtimeMs: 200 } as any);
    mockedFs.writeFileSync.mockClear();

    const cfg = configService.saveConfig({ claude: { seenModels: ['opus'] } as any });

    expect(cfg.ui.theme).toBe('nord'); // folded in the brain's write
    expect((cfg.claude as any).seenModels).toEqual(['opus']); // our partial applied on top
    expect(mockedFs.writeFileSync).toHaveBeenCalled(); // and persisted
  });
});

// ─── fail-safe on broken/unreadable config files ─────────────────────────────
// A YAML syntax error (or a transient read failure) must never wipe the user's
// config: no writeDefaults() over the file, saves blocked while broken, and the
// unparseable file backed up.

describe('loadFromDisk fail-safe — broken or unreadable config.yaml', () => {
  beforeEach(() => {
    mockedFs.readFileSync.mockReset().mockImplementation(() => enoent());
    mockedFs.writeFileSync.mockReset();
    mockedFs.copyFileSync.mockReset();
    mockedFs.mkdirSync.mockReset();
  });

  afterEach(() => {
    // Leave the singleton in the healthy first-run state for other suites.
    mockedFs.readFileSync.mockReset().mockImplementation(() => enoent());
    configService.reloadConfig();
  });

  it('ENOENT (first run) still seeds the file with defaults', () => {
    const cfg = configService.reloadConfig();
    expect(cfg.ui.theme).toBe('everforest');
    // writeDefaults ran: defaults were persisted for the first run.
    expect(mockedFs.writeFileSync).toHaveBeenCalledTimes(1);
  });

  it('a YAML parse error falls back to defaults WITHOUT overwriting the file', () => {
    mockedFs.readFileSync.mockReturnValue('ui:\n  theme: [unclosed');
    const cfg = configService.reloadConfig();

    // Defaults in memory…
    expect(cfg.ui.theme).toBe('everforest');
    // …but the broken file is never overwritten (no writeDefaults, no save).
    expect(mockedFs.writeFileSync).not.toHaveBeenCalled();
    // The unparseable file is backed up next to the original.
    expect(mockedFs.copyFileSync).toHaveBeenCalledTimes(1);
    const [src, dest] = mockedFs.copyFileSync.mock.calls[0] as [string, string];
    expect(String(dest)).toContain(`${src}.broken-`);
  });

  it('saveConfig refuses to persist while the on-disk config is broken', () => {
    mockedFs.readFileSync.mockReturnValue('ui:\n  theme: [unclosed');
    configService.reloadConfig();
    mockedFs.writeFileSync.mockClear();

    const cfg = configService.saveConfig({ ui: { theme: 'light' } as any });

    // The change applies in memory…
    expect(cfg.ui.theme).toBe('light');
    // …but nothing is written over the user's broken file.
    expect(mockedFs.writeFileSync).not.toHaveBeenCalled();
  });

  it('a non-ENOENT read error uses defaults in memory and blocks writes', () => {
    mockedFs.readFileSync.mockImplementation(() => {
      const err = new Error('EACCES: permission denied') as NodeJS.ErrnoException;
      err.code = 'EACCES';
      throw err;
    });
    const cfg = configService.reloadConfig();

    expect(cfg.ui.theme).toBe('everforest');
    // No writeDefaults (that's only for ENOENT), no backup (nothing readable).
    expect(mockedFs.writeFileSync).not.toHaveBeenCalled();
    expect(mockedFs.copyFileSync).not.toHaveBeenCalled();

    configService.saveConfig({ ui: { theme: 'light' } as any });
    expect(mockedFs.writeFileSync).not.toHaveBeenCalled();
  });

  it('a successful reload clears the block and saves persist again', () => {
    mockedFs.readFileSync.mockReturnValueOnce('ui:\n  theme: [unclosed');
    configService.reloadConfig();

    // File fixed: parses fine now.
    mockedFs.readFileSync.mockReturnValue('ui:\n  theme: light\n');
    const cfg = configService.reloadConfig();
    expect(cfg.ui.theme).toBe('light');

    mockedFs.writeFileSync.mockClear();
    configService.saveConfig({ ui: { fontSize: 16 } as any });
    expect(mockedFs.writeFileSync).toHaveBeenCalledTimes(1);
  });
});
