import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor, renderHook, act } from '@testing-library/react';
import React from 'react';
import SpawnAgentDialog from '../../src/components/SpawnAgentDialog';
import SupervisorSection from '../../src/components/settings/SupervisorSection';
import { HandoffDialog } from '../../src/components/claude/HandoffDialog';
import AskPane from '../../src/panes/AskPane';
import {
  __resetProviderDetectionCache,
  useProviderDetection,
} from '../../src/hooks/useProviderDetection';
import type { Config } from '../../src/hooks/useConfig';
import type { AgentProvider } from '../../src/types/pane';

/** Which harness AskPane's config says the supervisor runs on. Per-test, so the
 *  "configured but uninstalled" case can set it without a second mock. */
let configProvider: AgentProvider = 'claude';

// AskPane is the only component here that reads config from context rather than
// from a prop; the rest take it as one.
vi.mock('../../src/hooks/useConfig', () => ({
  useConfig: () => ({ config: { agents: { managerProvider: configProvider } }, save: vi.fn() }),
}));

/**
 * Pickers offer harnesses that exist on THIS machine.
 *
 * Workspacer speaks five (claude/codex/copilot/opencode/pi) and most machines
 * have one or two; a picker that lists all five is listing four spawn failures.
 * The detection is not new — `provider:checkAll` has lit the Spawn dialog's
 * green dots for a while — these tests pin what the pickers now DO with it, and
 * in particular the two ways this can go wrong invisibly: hiding a harness on
 * an answer we don't have, and hiding one the config is still pointed at.
 */

const api = window.electronAPI as unknown as Record<string, ReturnType<typeof vi.fn>>;

const detection = (rows: Array<[string, boolean]>) =>
  rows.map(([provider, found]) => ({
    provider,
    found,
    resolvedPath: found ? `/usr/bin/${provider}` : null,
    customBin: '',
  }));

const CLAUDE_AND_CODEX_ONLY = detection([
  ['claude', true],
  ['codex', true],
  ['copilot', false],
  ['opencode', false],
  ['pi', false],
]);

const CLAUDE_ONLY = detection([
  ['claude', true],
  ['codex', false],
  ['copilot', false],
  ['opencode', false],
  ['pi', false],
]);

function stubSpawnDialogApi() {
  api.claudeListModels = vi.fn().mockResolvedValue({
    defaultModel: '',
    skipPermissionsDefault: false,
    defaultPermissionMode: '',
    aliases: [],
    seen: [],
  });
  api.claudeProfilesList = vi.fn().mockResolvedValue([]);
  api.providerListModels = vi.fn().mockResolvedValue([]);
}

beforeEach(() => {
  __resetProviderDetectionCache();
  configProvider = 'claude';
  stubSpawnDialogApi();
  Object.defineProperty(window, 'localStorage', {
    configurable: true,
    value: {
      getItem: vi.fn(() => null),
      setItem: vi.fn(),
      removeItem: vi.fn(),
    },
  });
});

const providerCard = (name: RegExp) => screen.queryByRole('button', { name });

describe('Spawn dialog provider cards', () => {
  it('offers only the harnesses that are installed', async () => {
    api.providerCheckAll = vi.fn().mockResolvedValue(CLAUDE_AND_CODEX_ONLY);
    render(<SpawnAgentDialog defaultCwd="/repo" onSpawn={vi.fn()} onCancel={vi.fn()} />);

    await waitFor(() => expect(providerCard(/OpenCode/)).toBeNull());
    expect(providerCard(/Claude Code/)).toBeTruthy();
    expect(providerCard(/^Codex$/)).toBeTruthy();
    expect(providerCard(/GitHub Copilot/)).toBeNull();
    expect(providerCard(/^Pi/)).toBeNull();
  });

  it('collapses the row entirely when claude is the only harness', async () => {
    api.providerCheckAll = vi.fn().mockResolvedValue(CLAUDE_ONLY);
    render(<SpawnAgentDialog defaultCwd="/repo" onSpawn={vi.fn()} onCancel={vi.fn()} />);

    // One card is not a choice — the whole row goes, including the Claude card
    // that would otherwise sit there permanently selected.
    await waitFor(() => expect(providerCard(/^Codex$/)).toBeNull());
    expect(providerCard(/Claude Code/)).toBeNull();
  });

  it('keeps a missing harness visible when it is the pre-selected one, flagged', async () => {
    api.providerCheckAll = vi.fn().mockResolvedValue(CLAUDE_ONLY);
    render(
      <SpawnAgentDialog
        defaultCwd="/repo"
        defaultProvider="opencode"
        onSpawn={vi.fn()}
        onCancel={vi.fn()}
      />,
    );

    // Dropping it would render the picker with nothing selected, which reads as
    // a reset rather than as the diagnosis it is.
    await waitFor(() => expect(screen.getByText('NOT INSTALLED')).toBeTruthy());
    expect(providerCard(/OpenCode/)).toBeTruthy();
  });

  it('shows every harness while detection has not answered yet', () => {
    // A promise that never settles: the picker must not hide anything on the
    // strength of an answer it does not have.
    api.providerCheckAll = vi.fn().mockReturnValue(new Promise(() => {}));
    render(<SpawnAgentDialog defaultCwd="/repo" onSpawn={vi.fn()} onCancel={vi.fn()} />);

    expect(providerCard(/Claude Code/)).toBeTruthy();
    expect(providerCard(/OpenCode/)).toBeTruthy();
    expect(providerCard(/^Pi/)).toBeTruthy();
  });

  it('shows every harness when the host does not answer detection at all', async () => {
    api.providerCheckAll = vi.fn().mockRejectedValue(new Error('no such capability'));
    render(<SpawnAgentDialog defaultCwd="/repo" onSpawn={vi.fn()} onCancel={vi.fn()} />);

    await waitFor(() => expect(api.providerCheckAll).toHaveBeenCalled());
    expect(providerCard(/OpenCode/)).toBeTruthy();
  });
});

describe('Settings -> Fleet Manager harness picker', () => {
  const renderSupervisor = (config: Partial<Config>) =>
    render(
      <SupervisorSection
        config={config as Config}
        save={vi.fn().mockResolvedValue(config as Config)}
      />,
    );

  it('drops uninstalled harnesses from the manager picker', async () => {
    api.providerCheckAll = vi.fn().mockResolvedValue(CLAUDE_AND_CODEX_ONLY);
    renderSupervisor({ agents: { managerProvider: 'claude' } } as Partial<Config>);

    await waitFor(() => expect(screen.queryByRole('button', { name: 'OpenCode' })).toBeNull());
    expect(screen.getAllByRole('button', { name: 'Claude' }).length).toBeGreaterThan(0);
    expect(screen.queryByRole('button', { name: 'GitHub Copilot' })).toBeNull();
  });

  it('keeps the CONFIGURED harness listed and warns when its CLI is gone', async () => {
    api.providerCheckAll = vi.fn().mockResolvedValue(CLAUDE_ONLY);
    renderSupervisor({ agents: { managerProvider: 'codex' } } as Partial<Config>);

    // Same philosophy as the stale-model warning right below it in this
    // section: say what is wrong instead of quietly changing what is shown.
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Codex (not installed)' })).toBeTruthy(),
    );
    expect(screen.getByText(/was not found on this machine/)).toBeTruthy();
  });
});

describe('Handoff target picker', () => {
  const renderHandoff = (provider: AgentProvider) =>
    render(
      <HandoffDialog
        provider={provider}
        snapshot={{ sessionId: 's1', cwd: '/repo', conversation: [] } as never}
        cwd="/repo"
        busy={null}
        onCancel={vi.fn()}
        onConfirm={vi.fn()}
      />,
    );

  it('offers only installed harnesses as handoff targets', async () => {
    // A successor spawned on a CLI that isn't there dies on argv, and the
    // handoff has already spent a turn writing the brief by then.
    api.providerCheckAll = vi.fn().mockResolvedValue(CLAUDE_AND_CODEX_ONLY);
    renderHandoff('claude');

    await waitFor(() => expect(screen.queryByRole('button', { name: /OpenCode/ })).toBeNull());
    expect(screen.getByRole('button', { name: /Claude Code/ })).toBeTruthy();
    expect(screen.getByRole('button', { name: /Codex/ })).toBeTruthy();
    expect(screen.queryByRole('button', { name: /^Pi/ })).toBeNull();
  });

  it('keeps the SOURCE session’s harness even when detection missed it', async () => {
    // The session is demonstrably running on opencode — dropping it would
    // remove the same-harness "fresh context" handoff, which is the dialog's
    // most common use, and leave the target picker with nothing selected.
    api.providerCheckAll = vi.fn().mockResolvedValue(CLAUDE_ONLY);
    renderHandoff('opencode');

    await waitFor(() => expect(screen.queryByRole('button', { name: /Codex/ })).toBeNull());
    expect(screen.getByRole('button', { name: /OpenCode/ })).toBeTruthy();
  });
});

describe('Ask the Fleet harness picker', () => {
  const renderAsk = () =>
    render(<AskPane agents={[]} spawnAskAgent={vi.fn()} onJumpToAgent={vi.fn()} />);

  it('offers only installed harnesses to run the ask agent on', async () => {
    api.providerCheckAll = vi.fn().mockResolvedValue(CLAUDE_AND_CODEX_ONLY);
    renderAsk();

    await waitFor(() => expect(screen.queryByRole('button', { name: /OpenCode/ })).toBeNull());
    expect(screen.getByRole('button', { name: /Claude/ })).toBeTruthy();
    expect(screen.getByRole('button', { name: /Codex/ })).toBeTruthy();
  });

  it('hides the whole row when claude is the only harness', async () => {
    // With one harness there is nothing to pick, so the "Run on" row goes
    // rather than sitting there as a permanently-selected decoration.
    api.providerCheckAll = vi.fn().mockResolvedValue(CLAUDE_ONLY);
    renderAsk();

    await waitFor(() => expect(screen.queryByRole('button', { name: /Codex/ })).toBeNull());
    expect(screen.queryByText('Run on')).toBeNull();
    expect(screen.queryByRole('button', { name: /^Claude$/ })).toBeNull();
  });

  it('keeps the configured fleet harness listed and flagged when it is gone', async () => {
    // agents.managerProvider is what "Ask the Fleet" will actually launch;
    // silently offering something else would spawn a harness the user never
    // chose. Flag it instead.
    configProvider = 'opencode';
    api.providerCheckAll = vi.fn().mockResolvedValue(CLAUDE_ONLY);
    renderAsk();

    await waitFor(() =>
      expect(screen.getByRole('button', { name: /OpenCode \(not installed\)/ })).toBeTruthy(),
    );
  });
});

describe('the shared detection store', () => {
  it('serves a second picker from cache but forces a rescan on an explicit re-check', async () => {
    // Two TTLs sit in front of the PATH walk (this store's, and main's own in
    // agentProviders.checkAllProvidersCached), so a picker reopening is free —
    // but "re-check" is a person who just installed the CLI asking us to look
    // again, and neither TTL may answer for them.
    api.providerCheckAll = vi.fn().mockResolvedValue(CLAUDE_ONLY);
    const { result } = renderHook(() => useProviderDetection());
    await waitFor(() => expect(result.current.detection).toBeTruthy());
    expect(api.providerCheckAll).toHaveBeenCalledTimes(1);
    expect(api.providerCheckAll).toHaveBeenLastCalledWith(false);

    // A second picker opening inside the TTL asks nobody.
    renderHook(() => useProviderDetection());
    expect(api.providerCheckAll).toHaveBeenCalledTimes(1);

    api.providerCheckAll.mockResolvedValue(CLAUDE_AND_CODEX_ONLY);
    act(() => result.current.refresh());
    await waitFor(() =>
      expect(result.current.detection?.find((d) => d.provider === 'codex')?.found).toBe(true),
    );
    expect(api.providerCheckAll).toHaveBeenLastCalledWith(true);
  });
});
