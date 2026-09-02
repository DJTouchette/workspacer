import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { WHOLESALE_CONFIG_PATHS } from '../shared/configWholesale';
import wholesaleFixture from '../../../../../contracts/wholesale-config-paths.json';

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
  // withConfigLock (O_EXCL create + release) wraps every config write now, so
  // its primitives must exist too. openSync returning a fd = the lock is free.
  openSync: vi.fn().mockReturnValue(3),
  writeSync: vi.fn(),
  closeSync: vi.fn(),
  // Default: no file → mtime gate stays inert (configMtimeMs returns 0), so the
  // existing in-memory-cache tests behave exactly as before. The mtime-gate
  // suite below drives statSync explicitly.
  statSync: vi.fn().mockImplementation(() => enoent()),
}));

// Import after the mock is registered so the ConfigService constructor sees it.
// Because vitest hoists vi.mock, this import runs after the mock.
import * as fsMock from 'fs';
import {
  configService,
  ConfigService,
  deepMerge,
  applyWholesale,
  WholesaleValueError,
  setPreWriteHookForTest,
} from './configService';

const mockedFs = vi.mocked(fsMock);

describe('deepMerge semantics – via configService.saveConfig', () => {
  beforeEach(() => {
    // Reset to defaults between tests by reloading from a config.yaml that
    // parses to an empty object, so deepMerge(defaultConfig(), {}) hands back
    // a fresh defaults object every time. This is NOT an ENOENT reload: once
    // the singleton has ever held a config (true from module import onward),
    // loadFromDisk treats ENOENT as a mid-run disappearance and keeps the
    // existing (possibly test-polluted) config rather than reseeding — the
    // exact behavior the empty/comment-only-file fail-safe below depends on.
    mockedFs.readFileSync.mockReturnValue('{}\n');
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

  it('normalizes a legacy default model in memory and writes the canonical pair on save', () => {
    mockedFs.readFileSync.mockReturnValue(
      'customTop: keep-me\nclaude:\n  defaultModel: opus[1m]\n  seenModels: [claude-opus-5-1m]\n',
    );
    const loaded = configService.reloadConfig() as any;
    expect(loaded.claude).toMatchObject({
      defaultModel: 'opus',
      contextWindow: 1_000_000,
      seenModels: ['claude-opus-5'],
    });
    expect(loaded.customTop).toBe('keep-me');

    mockedFs.writeFileSync.mockClear();
    configService.saveConfig({ ui: { theme: 'light' } as any });
    const write = mockedFs.writeFileSync.mock.calls.find(
      ([, data]) => typeof data === 'string' && data.includes('defaultModel:'),
    );
    expect(write).toBeDefined();
    const text = String(write![1]);
    expect(text).toContain('defaultModel: opus');
    expect(text).toContain('contextWindow: 1000000');
    expect(text).toContain('customTop: keep-me');
    expect(text).not.toContain('opus[1m]');
    expect(text).not.toContain('opus-1m');
  });

  it('refuses a conflicting legacy marker and explicit config window', () => {
    mockedFs.readFileSync.mockReturnValue(
      'ui:\n  theme: light\nclaude:\n  defaultModel: opus[1m]\n  contextWindow: 200000\n',
    );
    mockedFs.copyFileSync.mockClear();
    const cfg = configService.reloadConfig() as any;
    expect(cfg.claude.defaultModel).toBe('opus');
    expect(cfg.claude.contextWindow).toBe(1_000_000);
    expect(cfg.ui.theme).toBe('light');
    expect(mockedFs.copyFileSync).not.toHaveBeenCalled();
  });

  it.each([
    ['null', 'null'],
    ['non-string', '42'],
  ])(
    'treats a %s defaultModel as selection validation, preserving YAML and future saves',
    (_name, yamlValue) => {
      mockedFs.readFileSync.mockReturnValue(
        `customTop: keep-me\nui:\n  theme: light\nclaude:\n  defaultModel: ${yamlValue}\n`,
      );
      mockedFs.copyFileSync.mockClear();

      const cfg = configService.reloadConfig() as any;

      expect(cfg.customTop).toBe('keep-me');
      expect(cfg.ui.theme).toBe('light');
      expect(cfg.claude.defaultModel).toBe('opus');
      expect(cfg.claude.contextWindow).toBe(1_000_000);
      expect(mockedFs.copyFileSync).not.toHaveBeenCalled();

      // Valid YAML with one invalid pair must not latch persistBlocked. The
      // next unrelated save still lands and preserves the rest of the file.
      mockedFs.writeFileSync.mockClear();
      configService.saveConfig({ ui: { fontSize: 16 } as any });
      expect(mockedFs.writeFileSync).toHaveBeenCalled();
      const written = String(mockedFs.writeFileSync.mock.calls.at(-1)?.[1] ?? '');
      expect(written).toContain('customTop: keep-me');
      expect(written).toContain('theme: light');
      expect(written).toContain('fontSize: 16');
    },
  );

  it('migrates a legacy Fleet Manager Claude marker into the per-provider context map', () => {
    mockedFs.readFileSync.mockReturnValue(
      'customTop: keep-me\nagents:\n  managerModels:\n    claude: opus[1m]\n    codex: gpt-5-codex\n',
    );
    const cfg = configService.reloadConfig() as any;
    expect(cfg.agents.managerModels).toEqual({ claude: 'opus', codex: 'gpt-5-codex' });
    expect(cfg.agents.managerContextWindows).toEqual({ claude: 1_000_000 });
    expect(cfg.customTop).toBe('keep-me');
  });

  it('round-trips explicit Codex provider-default null without erasing Claude', () => {
    mockedFs.readFileSync.mockReturnValue(
      'agents:\n  managerModels:\n    claude: opus\n    codex: gpt-5-codex\n  managerContextWindows:\n    claude: 1000000\n    codex: 400000\n',
    );
    configService.reloadConfig();
    mockedFs.writeFileSync.mockClear();
    const cfg = configService.saveConfig({
      agents: { managerContextWindows: { codex: null } } as any,
    }) as any;
    expect(cfg.agents.managerContextWindows).toEqual({ claude: 1_000_000, codex: null });
    const written = String(mockedFs.writeFileSync.mock.calls.at(-1)?.[1] ?? '');
    expect(written).toContain('claude: 1000000');
    expect(written).toContain('codex: null');
  });

  it('rejects a persisted context request for a provider-managed harness', () => {
    expect(() =>
      configService.saveConfig({
        agents: { managerContextWindows: { copilot: 1_000_000 } } as any,
      }),
    ).toThrow(/provider-managed/);
  });

  it('drops only an invalid old provider-managed context entry on read', () => {
    mockedFs.readFileSync.mockReturnValue(
      'ui:\n  theme: light\nagents:\n  managerContextWindows:\n    codex: 400000\n    copilot: 1000000\n',
    );
    const cfg = configService.reloadConfig() as any;
    expect(cfg.ui.theme).toBe('light');
    expect(cfg.agents.managerContextWindows).toEqual({ codex: 400_000 });
  });

  it('preserves unrelated top-level sections when saving a partial', () => {
    configService.saveConfig({ scripts: { '/repo': [{ name: 'x', command: 'y' }] } as any });
    const cfg = configService.getConfig();

    // scripts changed
    expect((cfg.scripts as any)['/repo'][0].name).toBe('x');
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

  it('saves a widget board keyed by cwd without touching other top-level keys', () => {
    const widgets = {
      '/home/user/proj': [
        { widget: 'git', size: 'large' as const },
        { plugin: 'djtouchette.shiplight', widget: 'lamp', size: 'small' as const },
      ],
    };
    configService.saveConfig({ widgets });
    const cfg = configService.getConfig();

    expect(cfg.widgets).toEqual(widgets);
    expect(cfg.scripts).toEqual({}); // the sibling per-directory map is untouched
    expect(cfg.terminal.fontSize).toBe(14);
  });

  // Boards are replaced wholesale, like scripts and customThemes: removing the
  // last widget from a project has to persist as an empty array, not merge back
  // into the previous contents.
  it('replaces a project board wholesale — removing the last widget persists', () => {
    configService.saveConfig({
      widgets: { '/home/user/proj': [{ widget: 'git', size: 'small' as const }] },
    });
    configService.saveConfig({ widgets: { '/home/user/proj': [] } });
    expect(configService.getConfig().widgets['/home/user/proj']).toEqual([]);
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
    // The retired fleet-supervisor block is gone from the canonical defaults.
    expect(brainDefaults).not.toHaveProperty('supervisor');
  });

  it('declares usage.pollOnBoot, on, in all three copies', () => {
    // The key travels to claudemon as an env var computed from the merged
    // config at spawn, so a default missing from any one copy is a daemon
    // spawned with the wrong answer by whichever runtime read that copy.
    expect(brainDefaults).toHaveProperty('usage.pollOnBoot', true);
    expect(CONFIG_DEFAULTS).toHaveProperty('usage.pollOnBoot', true);
    expect(RENDERER_CONFIG_DEFAULTS).toHaveProperty('usage.pollOnBoot', true);
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

// ─── save_config fix: CAS against a non-lock-participating writer ───────────
// The mtime-gate suite above proves saveConfig folds in an external write that
// landed BEFORE the call started. This proves the narrower, harder case: a
// write that lands DURING this call's own computation — after its initial read,
// before its write — from a writer that never took the cross-process lock (a
// hand edit, or this process's own unlocked migration writes). Mirrors
// briefService.test.ts's "COMPARE-AND-SWAP: an outside writer that ignores the
// lock is not overwritten" and the Go twin's TestConfigSaveCASRetriesAgainst...
describe('compare-and-swap — a writer that lands mid-save and does not hold the lock', () => {
  beforeEach(() => {
    mockedFs.readFileSync.mockReset();
    mockedFs.writeFileSync.mockReset();
    vi.mocked(fsMock.statSync).mockReset();
  });

  afterEach(() => {
    setPreWriteHookForTest(() => {});
    mockedFs.readFileSync.mockReset().mockImplementation(() => enoent());
    vi.mocked(fsMock.statSync)
      .mockReset()
      .mockImplementation(() => enoent());
    configService.reloadConfig();
  });

  it('retries against the fresh state and keeps BOTH changes', () => {
    mockedFs.readFileSync.mockReturnValue('ui:\n  theme: dark\n');
    vi.mocked(fsMock.statSync).mockReturnValue({ mtimeMs: 100, size: 20 } as any);
    configService.reloadConfig();

    let fired = false;
    setPreWriteHookForTest(() => {
      if (fired) return; // only the first attempt simulates the outsider
      fired = true;
      // The outsider writes directly to disk, bypassing withConfigLock
      // entirely — landing a change the read at the top of THIS attempt never
      // saw. Different section from our own partial, so an ordinary deep
      // merge (not a wholesale replace) is what has to notice and keep it.
      mockedFs.readFileSync.mockReturnValue('ui:\n  theme: nord\n');
      vi.mocked(fsMock.statSync).mockReturnValue({ mtimeMs: 200, size: 21 } as any);
    });

    const cfg = configService.saveConfig({ claude: { seenModels: ['opus'] } as any });

    expect(fired).toBe(true);
    expect(cfg.ui.theme).toBe('nord'); // the outsider's change survived
    expect((cfg.claude as any).seenModels).toEqual(['opus']); // ours did too
    expect(mockedFs.writeFileSync).toHaveBeenCalledTimes(1); // one write, not one per attempt
  });

  it('gives up (writing nothing) when an outsider churns the file on every attempt', () => {
    mockedFs.readFileSync.mockReturnValue('ui:\n  theme: dark\n');
    vi.mocked(fsMock.statSync).mockReturnValue({ mtimeMs: 100, size: 20 } as any);
    configService.reloadConfig();

    let n = 0;
    setPreWriteHookForTest(() => {
      n++;
      mockedFs.readFileSync.mockReturnValue(`ui:\n  theme: churn-${n}\n`);
      vi.mocked(fsMock.statSync).mockReturnValue({ mtimeMs: 100 + n, size: 20 + n } as any);
    });

    const cfg = configService.saveConfig({ claude: { seenModels: ['opus'] } as any });

    // Refused: the returned value is whatever the last fold-in saw, never
    // written, and definitely not OUR merge presented as applied.
    expect(mockedFs.writeFileSync).not.toHaveBeenCalled();
    expect((cfg.claude as any)?.seenModels).not.toEqual(['opus']);
  });
});

// ─── save_config fix: object values round-trip as objects ───────────────────
// The reported defect: an object-valued setting sent through save_config came
// back stringified rather than as an object. Exercised end to end through
// saveConfig -> getConfig, the same pair the MCP facade's save_config /
// get_config tools sit on top of.
describe('save_config: an object-valued setting round-trips as an object', () => {
  beforeEach(() => {
    // A config.yaml that parses to {} resets to fresh defaults via the normal
    // success path. A plain ENOENT would NOT reset here — the singleton has
    // already held a config since module import, so loadFromDisk now treats
    // ENOENT as a mid-run disappearance and keeps whatever config the prior
    // test left behind instead of reseeding defaults.
    mockedFs.readFileSync.mockReset().mockReturnValue('{}\n');
    mockedFs.writeFileSync.mockReset();
    vi.mocked(fsMock.statSync)
      .mockReset()
      .mockImplementation(() => enoent());
    configService.reloadConfig();
  });

  it('a nested object survives saveConfig -> getConfig as an object, not a string', () => {
    const cfg = configService.saveConfig({
      agents: { fleetFullAccess: true, managerProvider: 'claude' } as any,
      projects: { '/home/u/proj': { label: 'Proj', yolo: true } } as any,
    });

    expect(typeof cfg.agents).toBe('object');
    expect((cfg.agents as any).fleetFullAccess).toBe(true);
    expect(typeof (cfg.projects as any)['/home/u/proj']).toBe('object');
    expect((cfg.projects as any)['/home/u/proj'].yolo).toBe(true);

    // And the bytes actually written to disk are YAML, never a JSON-encoded
    // string masquerading as the value of a key.
    const written = String(mockedFs.writeFileSync.mock.calls.at(-1)?.[1] ?? '');
    expect(written).not.toMatch(/fleetFullAccess:\s*['"]/); // not quoted-as-string
    expect(written).toContain('fleetFullAccess: true');
    expect(written).toContain('yolo: true');
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

  it('a genuine first run (no config loaded yet) seeds the file with defaults', () => {
    // The shared singleton already has a config in memory, so reloadConfig()
    // on it can never exercise "first run" — that state (this.config still
    // undefined) only exists once, before a ConfigService's very first load.
    // Construct a fresh instance directly (same mocked fs, no module
    // reimport) to observe that state honestly.
    const fresh = new ConfigService();
    const cfg = fresh.getConfig();

    expect(cfg.ui.theme).toBe('everforest');
    // writeDefaults ran: defaults were persisted for the first run.
    expect(mockedFs.writeFileSync).toHaveBeenCalledTimes(1);
  });

  it('ENOENT after a config was already loaded is NOT first run — no reseed, saves blocked', () => {
    // The shared singleton already has a config (from module import). A
    // disappearing config.yaml here is a mid-run disappearance (e.g. a hand
    // edit that truncated before rewriting), not "no config yet" — it must
    // NOT be reseeded with bare defaults, which would then be written over
    // whatever the user actually has on the next save.
    const before = configService.getConfig();
    const cfg = configService.reloadConfig();

    expect(cfg).toBe(before); // kept the existing in-memory config, not fresh defaults
    expect(mockedFs.writeFileSync).not.toHaveBeenCalled();

    mockedFs.writeFileSync.mockClear();
    configService.saveConfig({ ui: { theme: 'light' } as any });
    expect(mockedFs.writeFileSync).not.toHaveBeenCalled();
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

  it.each([
    ['an empty file', ''],
    ['a comment-only file', '# just a comment\n'],
  ])(
    '%s parses to a non-object — refuses to persist instead of resetting to defaults',
    (_label, contents) => {
      // yaml.load('') / yaml.load('# comment') return undefined/null, not a
      // parse error. That used to slip past the guard entirely: deepMerge(defaults,
      // undefined) silently returns defaults with no persistBlocked, no backup, no
      // log — and the NEXT save (which re-reads unconditionally) would then write
      // those bare defaults over the user's real config. The property under test
      // is the refusal, not just the in-memory parse result.
      mockedFs.readFileSync.mockReturnValue(contents);
      const cfg = configService.reloadConfig();

      // Defaults in memory…
      expect(cfg.ui.theme).toBe('everforest');
      // …but nothing was written to "fix" the file.
      expect(mockedFs.writeFileSync).not.toHaveBeenCalled();

      // And the fail-safe actually blocks the next save, rather than letting it
      // through to reset the real file to defaults.
      mockedFs.writeFileSync.mockClear();
      const saved = configService.saveConfig({ ui: { theme: 'light' } as any });
      expect(mockedFs.writeFileSync).not.toHaveBeenCalled();
      // The change is kept in memory only (saveConfigLocked's existing
      // persistBlocked behavior), not silently dropped.
      expect(saved.ui.theme).toBe('light');
    },
  );

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

// ─── the save FAILURE plane ──────────────────────────────────────────────────
// What the caller is told when the bytes do not reach disk. Whatever saveConfig
// returns is what IPC.CONFIG_SAVE hands back to ConfigContext.save, which does
// setConfig(cfg) — so returning a value that is not on disk IS the success
// report, and the Settings pane paints the setting as applied until the next
// restart silently reverts it.
//
// Twinned with services/hub/cmd/brain/config_writefailure_test.go: the brain is
// the provider of config.save in the shipped default (bus mode + catalog
// delegation), this copy is the provider with WORKSPACER_NO_BRAIN=1 /
// delegation off, and it is ALWAYS the writer for main's own saves
// (usageAccumulator seenModels, budgets, keepWarm).

describe('save failure plane — a write that never reached disk', () => {
  beforeEach(() => {
    mockedFs.readFileSync.mockReset().mockReturnValue('ui:\n  theme: everforest\n');
    mockedFs.writeFileSync.mockReset();
    mockedFs.renameSync.mockReset();
    mockedFs.copyFileSync.mockReset();
    vi.mocked(fsMock.openSync)
      .mockReset()
      .mockReturnValue(3 as any);
    vi.mocked(fsMock.statSync)
      .mockReset()
      .mockReturnValue({ mtimeMs: 100, size: 26 } as any);
    configService.reloadConfig();
  });

  afterEach(() => {
    mockedFs.readFileSync.mockReset().mockImplementation(() => enoent());
    vi.mocked(fsMock.statSync)
      .mockReset()
      .mockImplementation(() => enoent());
    vi.mocked(fsMock.openSync)
      .mockReset()
      .mockReturnValue(3 as any);
    configService.reloadConfig();
  });

  it('returns the OLD value — not the merged one — when the write throws ENOSPC', () => {
    expect(configService.getConfig().ui.theme).toBe('everforest');
    mockedFs.writeFileSync.mockImplementation(() => {
      const err = new Error('ENOSPC: no space left on device') as NodeJS.ErrnoException;
      err.code = 'ENOSPC';
      throw err;
    });

    const returned = configService.saveConfig({ ui: { theme: 'nord' } as any });

    expect(returned.ui.theme).toBe('everforest');
  });

  it('does not ADOPT the value it failed to write (getConfig keeps serving the old one)', () => {
    mockedFs.writeFileSync.mockImplementation(() => {
      const err = new Error('EIO') as NodeJS.ErrnoException;
      err.code = 'EIO';
      throw err;
    });
    configService.saveConfig({ ui: { theme: 'nord' } as any });

    // The write failed, so the file (and its stamp) never moved: a phantom in
    // the cache would be served for the life of the main process — to spawn
    // defaults, notifications, budgets and keepWarm as well as Settings.
    expect(configService.getConfig().ui.theme).toBe('everforest');
  });

  it('refuses the save AND reports the old value when the other writer holds the lock', () => {
    // openSync EEXIST = the lock file is there; statSync gives it a fresh mtime
    // so it is held-not-stale and cannot be stolen.
    vi.mocked(fsMock.openSync).mockImplementation(() => {
      const err = new Error('EEXIST: file already exists') as NodeJS.ErrnoException;
      err.code = 'EEXIST';
      throw err;
    });
    vi.mocked(fsMock.statSync).mockReturnValue({
      mtimeMs: Date.now(),
      size: 26,
      mtime: new Date(),
    } as any);
    mockedFs.writeFileSync.mockClear();

    const returned = configService.saveConfig({ ui: { theme: 'nord' } as any });

    // Writing anyway is the data-loss bug the cross-process lock exists to
    // prevent…
    expect(mockedFs.writeFileSync).not.toHaveBeenCalled();
    // …and reporting the caller its own value would make a refused save
    // indistinguishable from an applied one.
    expect(returned.ui.theme).toBe('everforest');
  });

  it('getConfig sees an external write that changed only the LENGTH, not the mtime', () => {
    expect(configService.getConfig().ui.theme).toBe('everforest');
    // The brain rewrites config.yaml in its own process and the write lands in
    // the same filesystem timestamp tick (1s granularity on ext4 with 128-byte
    // inodes, HFS+, NFSv3; 2s on FAT). An mtime-only gate is blind to it.
    mockedFs.readFileSync.mockReturnValue('ui:\n  theme: nord\n');
    vi.mocked(fsMock.statSync).mockReturnValue({ mtimeMs: 100, size: 19 } as any);

    expect(configService.getConfig().ui.theme).toBe('nord');
  });

  it('saveConfig folds in an external write the stamp cannot see at all', () => {
    // Same mtime AND same length as ours: no cheap stat can tell these apart,
    // so the read-modify-write under the cross-process lock must re-read
    // unconditionally rather than trusting the gate.
    mockedFs.readFileSync.mockReturnValue('ui:\n  theme: dark\n');
    vi.mocked(fsMock.statSync).mockReturnValue({ mtimeMs: 100, size: 19 } as any);
    configService.reloadConfig();
    configService.saveConfig({ claude: { seenModels: ['sonnet'] } as any });

    mockedFs.readFileSync.mockReturnValue('ui:\n  theme: nord\n');
    vi.mocked(fsMock.statSync).mockReturnValue({ mtimeMs: 100, size: 19 } as any);
    mockedFs.writeFileSync.mockClear();

    const cfg = configService.saveConfig({ claude: { seenModels: ['sonnet', 'opus'] } as any });

    expect(cfg.ui.theme).toBe('nord');
    const written = String(mockedFs.writeFileSync.mock.calls.at(-1)?.[1] ?? '');
    expect(written).toContain('theme: nord');
  });
});

// ─── wholesale paths ────────────────────────────────────────────────────────
// The renderer trims a save down to what actually changed (lib/configPatch) and
// must skip exactly the paths main replaces instead of merging. Those were two
// hand-kept lists and they drifted: `projects` was wholesale here but trimmed
// there, so saving an icon on one project shipped a one-entry map that main
// took as the whole truth — every other project lost its identity. One shared
// list now (main/shared/configWholesale); this pins every entry of it to the
// behaviour that justifies being on it.
//
// Driven through the pure deepMerge + applyWholesale pair rather than
// saveConfig: the fs mock above throws ENOENT on every read, so saveConfig
// reloads pure defaults each call and cannot hold the prior state that the
// resurrection hazard is about.
describe('wholesale config paths — deletion survives the merge', () => {
  const at = (obj: unknown, p: string): any =>
    p.split('.').reduce<any>((o, k) => o?.[k], obj as any);
  /** `{a: {b: v}}` from `'a.b'`. */
  const nest = (p: string, v: unknown): any =>
    p
      .split('.')
      .reverse()
      .reduce<unknown>((acc, k) => ({ [k]: acc }), v);

  // The it.each blocks below are driven BY the set, so dropping a path from it
  // would silently drop its coverage rather than fail. Pin the membership.
  it('covers every path main replaces wholesale', () => {
    expect([...WHOLESALE_CONFIG_PATHS].sort()).toEqual([
      'claude.budgets',
      'projects',
      'ui.customThemes',
    ]);
  });

  // Cross-language guard: the Go brain answers config.save for every
  // web/mobile/MCP save_config caller, and used to hand-special-case only
  // ui.customThemes and claude.budgets — `projects` was wholesale here and
  // deep-merged there, so a project delete sent through save_config silently
  // failed to delete. contracts/wholesale-config-paths.json pins both sides
  // to the same list (see cmd/brain config_wholesale_test.go for the Go half).
  it('agrees with the Go twin (contracts/wholesale-config-paths.json)', () => {
    expect([...WHOLESALE_CONFIG_PATHS].sort()).toEqual([...wholesaleFixture.paths].sort());
  });

  it.each([...WHOLESALE_CONFIG_PATHS])('%s', (dotted) => {
    const current = nest(dotted, { keep: { n: 1 }, drop: { n: 2 } });
    // The caller re-sends the surviving map — the only way to express a delete.
    const partial = nest(dotted, { keep: { n: 1 } });

    const merged = deepMerge(current, partial);
    // Deep merge alone resurrects `drop`: this is the hazard, not a typo.
    expect(at(merged, dotted)).toEqual({ keep: { n: 1 }, drop: { n: 2 } });

    applyWholesale(merged, partial, dotted);
    expect(at(merged, dotted)).toEqual({ keep: { n: 1 } });
  });

  it.each([...WHOLESALE_CONFIG_PATHS])('%s survives a save that omits it', (dotted) => {
    const current = nest(dotted, { keep: { n: 1 } });
    // Absent from the partial means "not touching this map" — emptying it here
    // would wipe the map on every unrelated setting change.
    const partial = { ui: { theme: 'nord' } };
    const merged = deepMerge(current, partial);
    applyWholesale(merged, partial, dotted);
    expect(at(merged, dotted)).toEqual({ keep: { n: 1 } });
  });

  it('empties the map when the caller sends an empty one', () => {
    const merged = deepMerge({ projects: { '/w/a': { label: 'A' } } }, { projects: {} });
    applyWholesale(merged, { projects: {} }, 'projects');
    expect(merged.projects).toEqual({});
  });

  it('ignores a same-named key at a different depth', () => {
    // supervisor.budgets is NOT claude.budgets and must merge normally.
    const merged = deepMerge(
      { supervisor: { budgets: { a: 1, b: 2 } } },
      { supervisor: { budgets: { a: 9 } } },
    );
    applyWholesale(merged, { supervisor: { budgets: { a: 9 } } }, 'claude.budgets');
    expect(merged.supervisor.budgets).toEqual({ a: 9, b: 2 });
  });
});

// ─── the wholesale VALUE contract (shared with the Go brain) ────────────────
// contracts/wholesale-config-paths.json's `valueCases` block is the SHARED
// fixture: services/hub/cmd/brain/config_wholesale_test.go's
// TestWholesaleValueContractCases runs the exact same rows through the Go
// applyWholesale.
//
// It exists because the two writers disagreed here, in opposite and equally
// wrong directions, on the one input that matters most. A wholesale path is
// REPLACED rather than merged, so the value's type is load-bearing: the Go
// brain coerced any non-object to `{}` — deleting the user's whole projects /
// customThemes / budgets map and reporting a SUCCESSFUL save, with no backup
// taken on a successful write — while this side wrote the bad value through
// verbatim (`dst[leaf] = src[leaf] ?? {}`), leaving a string where every reader
// indexes a map. Both refuse now, and the fixture is what stops them drifting
// apart again.
describe('wholesale VALUE contract — shared with the Go brain', () => {
  interface ValueCase {
    name: string;
    path: string;
    current: Record<string, unknown>;
    value: unknown;
    expect: 'accept' | 'refuse';
    expected?: Record<string, unknown>;
    refusedBy?: string;
  }
  const valueCases = (wholesaleFixture as unknown as { valueCases: ValueCase[] }).valueCases;

  const at = (obj: unknown, p: string): any =>
    p.split('.').reduce<any>((o, k) => o?.[k], obj as any);
  const nest = (p: string, v: unknown): any =>
    p
      .split('.')
      .reverse()
      .reduce<unknown>((acc, k) => ({ [k]: acc }), v);

  it('the fixture loads and carries refusals', () => {
    expect(Array.isArray(valueCases)).toBe(true);
    // The floor, and the SHAPE of the floor. A corpus that kept only its accept
    // cases would pass against the exact coercion this block exists to kill.
    expect(valueCases.length).toBeGreaterThanOrEqual(11);
    expect(valueCases.filter((c) => c.expect === 'refuse').length).toBeGreaterThan(0);
  });

  it('exercises every path main replaces wholesale', () => {
    const covered = new Set(valueCases.map((c) => c.path));
    for (const p of WHOLESALE_CONFIG_PATHS) expect(covered.has(p)).toBe(true);
  });

  // A local counter, not tests/support/sweepTally: that module probes the real
  // fs at import time and this file mocks fs module-wide. Same job — prove every
  // row ASSERTED, so a loader that silently enumerated nothing goes red.
  let executed = 0;
  for (const c of valueCases) {
    it(c.name, () => {
      executed++;
      const merged = nest(c.path, structuredClone(c.current));
      const partial = nest(c.path, structuredClone(c.value));
      if (c.expect === 'accept') {
        applyWholesale(merged, partial, c.path);
        expect(at(merged, c.path)).toEqual(c.expected);
        return;
      }
      expect(() => applyWholesale(merged, partial, c.path)).toThrow(WholesaleValueError);
      // A refusal must leave the document ALONE. Refusing and then emptying the
      // map anyway is the defect wearing an error.
      expect(at(merged, c.path)).toEqual(c.current);
    });
  }
  it('ran every row of the corpus', () => {
    expect(executed).toBe(valueCases.length);
    expect(executed).toBeGreaterThanOrEqual(11);
  });

  // TS-ONLY, and deliberately not in the fixture: JSON cannot carry `undefined`,
  // so the Go twin never sees one. In JavaScript it is how "I did not provide
  // this" is spelled — `{ projects: undefined }` is a caller not touching the
  // map, not a caller asking to empty it — so it is treated as absent rather
  // than refused. (It used to be `?? {}`, i.e. emptied.)
  it('treats an explicit undefined as absent, not as a refusal or a wipe', () => {
    const merged = { projects: { '/w/a': { label: 'A' } } };
    applyWholesale(merged, { projects: undefined }, 'projects');
    expect(merged.projects).toEqual({ '/w/a': { label: 'A' } });
  });
});

// ─── retired config keys are stripped on read ───────────────────────────────
// The unit-level rules (what a block is, what survives) live in
// lib/orphanedConfigKeys.test.ts. This is the WIRING: that loadFromDisk
// actually runs the pruner, that the config it hands out no longer carries the
// key, and that the file it writes back is the text splice — not a yaml.dump,
// which would drop the user's comments and re-order every key.
describe('retired config keys — the orphaned supervisor block', () => {
  beforeEach(() => {
    mockedFs.readFileSync.mockReset();
    mockedFs.writeFileSync.mockReset();
    vi.mocked(fsMock.statSync)
      .mockReset()
      .mockImplementation(() => enoent());
  });

  afterEach(() => {
    mockedFs.readFileSync.mockReset().mockImplementation(() => enoent());
    mockedFs.writeFileSync.mockReset();
    configService.reloadConfig();
  });

  /** The bytes atomicWriteFileSync put in its temp file (the config payload). */
  const writtenConfig = (): string | undefined => {
    const call = mockedFs.writeFileSync.mock.calls.at(-1);
    return call ? String(call[1]) : undefined;
  };

  it('drops the block from the config it hands out, and rewrites the file', () => {
    mockedFs.readFileSync.mockReturnValue(
      '# hand written\nui:\n  theme: nord\n\nsupervisor:\n  provider: claude\n  fullAccess: true\n\nclaude:\n  defaultModel: opus\n',
    );
    const cfg = configService.reloadConfig() as unknown as Record<string, unknown>;

    expect(cfg.supervisor).toBeUndefined();
    expect(cfg.ui.theme).toBe('nord');
    // Byte-for-byte the input minus the block: the leading comment survives,
    // key order survives, and nothing was re-serialized.
    expect(writtenConfig()).toBe(
      '# hand written\nui:\n  theme: nord\n\nclaude:\n  defaultModel: opus\n',
    );
  });

  it('writes nothing when the config has no such block (the new-install case)', () => {
    mockedFs.readFileSync.mockReturnValue('ui:\n  theme: nord\n');
    const cfg = configService.reloadConfig();

    expect(cfg.ui.theme).toBe('nord');
    expect(mockedFs.writeFileSync).not.toHaveBeenCalled();
  });

  it('leaves an unknown key that is NOT retired completely alone', () => {
    // The whole point of the named list: no general unknown-key pruning. A key
    // the defaults have never heard of (a plugin's, a late-loading feature's)
    // must round-trip untouched.
    mockedFs.readFileSync.mockReturnValue('ui:\n  theme: nord\npluginSettings:\n  a: 1\n');
    const cfg = configService.reloadConfig() as unknown as Record<string, unknown>;

    expect(cfg.pluginSettings).toEqual({ a: 1 });
    expect(mockedFs.writeFileSync).not.toHaveBeenCalled();
  });
});
