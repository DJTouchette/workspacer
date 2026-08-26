import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import React from 'react';
import { ComposerControls } from '../../src/components/claude/ComposerControls';
import type { ClaudeSessionSnapshot } from '../../src/types/claudeSession';
import type { AgentProvider } from '../../src/types/pane';

/**
 * ComposerControls renders the model / effort / permission pills and owns the
 * live-switch vs restart decision per provider (see lib/providerCaps.ts). The
 * pills must reflect the session's reported state, and the live-switch paths
 * must call the exact daemon endpoints — claudeMessage("/model …") for claude,
 * claudeSetPermissionMode for the permission mode.
 */

// These methods aren't in tests/setup.ts's base mock; add them so the pills can
// load model lists and drive live switches.
const api = window.electronAPI as unknown as Record<string, ReturnType<typeof vi.fn>>;

function snapshot(overrides: Partial<ClaudeSessionSnapshot> = {}): ClaudeSessionSnapshot {
  return {
    sessionId: 'sess-1',
    cwd: '/repo',
    status: 'active',
    conversation: [],
    activeToolCalls: [],
    completedToolCalls: [],
    fileChanges: [],
    pendingApproval: null,
    subagents: [],
    ambientState: 'idle',
    lastActivity: Date.now(),
    totalToolCalls: 0,
    ...overrides,
  } as ClaudeSessionSnapshot;
}

function renderControls(
  props: {
    provider?: AgentProvider;
    sessionId?: string | null;
    snapshot?: ClaudeSessionSnapshot | null;
  } = {},
) {
  const onRestartWith = vi.fn();
  render(
    <ComposerControls
      provider={props.provider ?? 'claude'}
      sessionId={props.sessionId === undefined ? 'sess-1' : props.sessionId}
      snapshot={props.snapshot ?? snapshot()}
      cwd="/repo"
      onRestartWith={onRestartWith}
    />,
  );
  return { onRestartWith };
}

beforeEach(() => {
  api.claudeListModels = vi.fn().mockResolvedValue({
    defaultModel: 'sonnet',
    skipPermissionsDefault: false,
    defaultPermissionMode: '',
    aliases: [
      { value: 'opus', label: 'Opus' },
      { value: 'sonnet', label: 'Sonnet' },
    ],
    seen: [],
  });
  api.providerListModels = vi.fn().mockResolvedValue([
    {
      id: 'gpt-5-codex',
      label: 'GPT-5 Codex',
      default: true,
      effortLevels: ['low', 'medium', 'high', 'xhigh'],
    },
    { id: 'o3', label: 'o3', default: false, effortLevels: ['low', 'high'] },
  ]);
  api.claudeSetModel = vi.fn().mockResolvedValue({ ok: true });
  api.claudeSetPermissionMode = vi.fn().mockResolvedValue({ ok: true });
  api.claudeSetEffort = vi.fn().mockResolvedValue({ ok: true, effort: 'high' });
  api.claudeMessage = vi.fn().mockResolvedValue({ ok: true });
});

afterEach(() => {
  vi.clearAllMocks();
});

describe('ComposerControls — pill labels reflect session state', () => {
  it('shows the live model from statusLine, and the spawn permission mode', () => {
    renderControls({
      snapshot: snapshot({
        statusLine: { modelDisplay: 'Opus 4.8' } as any,
        settings: { permissionMode: 'plan' },
      }),
    });
    expect(screen.getByText('Opus 4.8')).toBeInTheDocument();
    expect(screen.getByText('Plan mode')).toBeInTheDocument();
  });

  it('falls back to the spawn-requested model when no live telemetry has arrived', () => {
    renderControls({ snapshot: snapshot({ settings: { model: 'my-custom-model' } }) });
    expect(screen.getByText('my-custom-model')).toBeInTheDocument();
  });

  it('live permission mode (hook telemetry) wins over the frozen spawn setting', () => {
    renderControls({
      snapshot: snapshot({
        settings: { permissionMode: 'default' },
        livePermissionMode: 'acceptEdits',
      }),
    });
    expect(screen.getByText('Accept edits')).toBeInTheDocument();
    expect(screen.queryByText('Ask to approve')).not.toBeInTheDocument();
  });

  it('renders an effort pill for both codex and claude', () => {
    const { unmount } = render(
      <ComposerControls
        provider="codex"
        sessionId="s"
        snapshot={snapshot({ settings: { effort: 'high' } })}
        cwd="/r"
        onRestartWith={vi.fn()}
      />,
    );
    expect(screen.getByText('High')).toBeInTheDocument();
    unmount();
    render(
      <ComposerControls
        provider="claude"
        sessionId="s"
        // Claude's `--effort` ladder adds xhigh/max; the pill shows its label.
        snapshot={snapshot({ settings: { effort: 'xhigh' } })}
        cwd="/r"
        onRestartWith={vi.fn()}
      />,
    );
    expect(screen.getByText('Extra high')).toBeInTheDocument();
  });

  it('shows the harness default as the selected effort when none was overridden', async () => {
    renderControls({ provider: 'codex', snapshot: snapshot({ settings: {} }) });

    fireEvent.click(screen.getByText('Default'));
    // The current value's menu row wraps its label in a span with a trailing
    // <Check> icon (checkedLabel); the pill itself is a button, so filter on tag.
    const marked = (await screen.findAllByText('Default')).find(
      (el) => el.tagName === 'SPAN' && el.querySelector('svg'),
    );
    expect(marked).toBeTruthy();
  });

  it('disables the pills when there is no session yet', () => {
    renderControls({ sessionId: null });
    // Every pill is a disabled button in the no-session state.
    const buttons = screen.getAllByRole('button');
    expect(buttons.length).toBeGreaterThan(0);
    buttons.forEach((b) => expect(b).toBeDisabled());
  });
});

/**
 * The permission pill is a SAFETY indicator, and until 2026-08-26 it answered
 * an absent mode with the provider's first one — "Ask to approve". On the
 * remote path that was reliably the wrong answer: a full-access session spawned
 * from /app against a headless hub arrives as a sparse row carrying no
 * permission fields, so the pane showed maximum caution for a session running
 * with every approval bypassed. These pin the three states the bar has to tell
 * apart.
 */
describe('ComposerControls — the permission pill never invents a mode', () => {
  it('says Full access for a session the hub confirmed runs bypassed', () => {
    renderControls({
      provider: 'claude',
      // What webBackend folds on from the spawn result's `fullAccess: true`,
      // and what the brain overlays onto the sparse row for every other client.
      snapshot: snapshot({
        settings: { permissionMode: 'bypassPermissions', bypassAvailable: true },
      }),
    });
    expect(screen.getByText('Full access')).toBeInTheDocument();
    expect(screen.queryByText('Ask to approve')).not.toBeInTheDocument();
    expect(screen.queryByText('Unknown')).not.toBeInTheDocument();
  });

  it('says Unknown — not the provider default — when nothing reported the mode', () => {
    // A sparse row from a headless node the hub never watched spawn: no
    // livePermissionMode, no settings at all.
    renderControls({ provider: 'claude', snapshot: snapshot() });
    expect(screen.getByText('Unknown')).toBeInTheDocument();
    expect(screen.queryByText('Ask to approve')).not.toBeInTheDocument();
  });

  it('still lets an unknown session be switched, and says why it is unknown', async () => {
    renderControls({ provider: 'claude', snapshot: snapshot() });
    const pill = screen.getByText('Unknown').closest('button')!;
    expect(pill.title).toMatch(/unknown/i);
    // Admitting ignorance must not disable the control that fixes it.
    fireEvent.click(pill);
    fireEvent.click(await screen.findByText('Plan mode'));
    expect(api.claudeSetPermissionMode).toHaveBeenCalledWith('sess-1', 'plan');
  });

  it('uses the managed vocabulary for a managed provider', () => {
    renderControls({
      provider: 'codex',
      snapshot: snapshot({ settings: { permissionMode: 'yolo' } }),
    });
    expect(screen.getByText('Full access')).toBeInTheDocument();
  });
});

/**
 * NO SILENT DOWNGRADES. The hub answers every spawn with `escalationScrubbed` —
 * what was asked for and refused — and nothing in the UI read it: a clamped
 * full-access click looked exactly like an ask-mode spawn, and the only record
 * was a log line on the host. The chip is that record, put where the person who
 * clicked can see it.
 */
describe('ComposerControls — a refused escalation is shown, not logged', () => {
  it('flags a full-access request the hub clamped, beside the mode it settled on', () => {
    renderControls({
      provider: 'claude',
      snapshot: snapshot({
        settings: { permissionMode: 'default', bypassAvailable: false },
        escalationScrubbed: ['skipPermissions'],
      }),
    });
    // The pill tells the truth about what is running…
    expect(screen.getByText('Ask to approve')).toBeInTheDocument();
    // …and the chip tells the truth about what was asked for.
    const chip = screen.getByText('Full access refused');
    expect(chip).toBeInTheDocument();
    expect(chip.closest('span')?.title).toMatch(/running as/i);
  });

  it('names a non-permission refusal differently', () => {
    renderControls({
      provider: 'claude',
      snapshot: snapshot({
        settings: { permissionMode: 'default' },
        escalationScrubbed: ['profileId'],
      }),
    });
    expect(screen.getByText('Request refused')).toBeInTheDocument();
    expect(screen.getByText('Request refused').closest('span')?.title).toMatch(/account profile/i);
  });

  it('shows nothing at all when nothing was refused', () => {
    renderControls({
      provider: 'claude',
      snapshot: snapshot({ settings: { permissionMode: 'bypassPermissions' } }),
    });
    expect(screen.queryByText(/refused/i)).not.toBeInTheDocument();
  });
});

describe('ComposerControls — claude live switches', () => {
  it('switching a claude model sends "/model <id>" through the message path', async () => {
    renderControls({ provider: 'claude', snapshot: snapshot({ settings: { model: 'sonnet' } }) });
    // Open the model menu (the pill shows the current model label).
    fireEvent.click(screen.getByText('sonnet'));
    const opus = await screen.findByText('Opus');
    fireEvent.click(opus);
    expect(api.claudeMessage).toHaveBeenCalledWith('sess-1', '/model opus');
    // claude never routes a model change through the managed endpoint.
    expect(api.claudeSetModel).not.toHaveBeenCalled();
  });

  it('switching a claude permission mode calls claudeSetPermissionMode with the mode id', async () => {
    renderControls({
      provider: 'claude',
      snapshot: snapshot({ settings: { permissionMode: 'default' } }),
    });
    fireEvent.click(screen.getByText('Ask to approve'));
    const plan = await screen.findByText('Plan mode');
    fireEvent.click(plan);
    expect(api.claudeSetPermissionMode).toHaveBeenCalledWith('sess-1', 'plan');
  });
});

/**
 * Claude refuses to enter bypassPermissions mid-session unless the process was
 * launched with --dangerously-skip-permissions — verified against the stream
 * transport's control protocol, which answers "Cannot set permission mode to
 * bypassPermissions because the session was not launched with
 * --dangerously-skip-permissions". `settings.bypassAvailable` records that flag
 * at spawn so the pill can route the pick correctly instead of firing a request
 * that cannot succeed and rendering the CLI's error at the user.
 */
describe('ComposerControls — Full access is gated at launch', () => {
  it('routes Full access straight to the restart confirm when the session lacks the launch flag', async () => {
    const { onRestartWith } = renderControls({
      provider: 'claude',
      snapshot: snapshot({ settings: { permissionMode: 'default', bypassAvailable: false } }),
    });
    fireEvent.click(screen.getByText('Ask to approve'));
    fireEvent.click(await screen.findByText('Full access'));
    // No doomed round trip to the daemon…
    expect(api.claudeSetPermissionMode).not.toHaveBeenCalled();
    // …and the confirm says why, in prose.
    expect(await screen.findByText(/only be granted at launch/)).toBeInTheDocument();
    fireEvent.click(await screen.findByText(/Restart with Full access/));
    expect(onRestartWith).toHaveBeenCalledWith({ permissionMode: 'bypassPermissions' });
  });

  it('marks the row as restarting so the cost is visible before the click', async () => {
    renderControls({
      provider: 'claude',
      snapshot: snapshot({ settings: { permissionMode: 'default', bypassAvailable: false } }),
    });
    fireEvent.click(screen.getByText('Ask to approve'));
    expect(await screen.findByText('restarts')).toBeInTheDocument();
  });

  it('switches live when the session was launched with the flag', async () => {
    renderControls({
      provider: 'claude',
      snapshot: snapshot({
        settings: { permissionMode: 'acceptEdits', bypassAvailable: true },
      }),
    });
    fireEvent.click(screen.getByText('Accept edits'));
    fireEvent.click(await screen.findByText('Full access'));
    expect(api.claudeSetPermissionMode).toHaveBeenCalledWith('sess-1', 'bypassPermissions');
    expect(screen.queryByText('restarts')).not.toBeInTheDocument();
  });

  it('still asks the daemon when the launch is unrecorded (restored session)', async () => {
    renderControls({
      provider: 'claude',
      snapshot: snapshot({ settings: { permissionMode: 'default' } }),
    });
    fireEvent.click(screen.getByText('Ask to approve'));
    fireEvent.click(await screen.findByText('Full access'));
    expect(api.claudeSetPermissionMode).toHaveBeenCalledWith('sess-1', 'bypassPermissions');
  });
});

/**
 * "Default" effort means "pass no --effort", but the level it lands on is
 * knowable and used to be hidden behind the word. Claude's comes from its
 * settings chain (resolved at spawn into settings.defaultEffort, because the CLI
 * reports the effective level in no telemetry channel); Codex reports it per
 * model on the live catalog row (`defaultReasoningEffort`).
 */
describe('ComposerControls — Default effort names the level it resolves to', () => {
  it('claude: the pill shows the resolved level instead of the word', () => {
    renderControls({
      provider: 'claude',
      snapshot: snapshot({ settings: { defaultEffort: 'high' } }),
    });
    expect(screen.getByText('High')).toBeInTheDocument();
    expect(screen.queryByText('Default')).not.toBeInTheDocument();
  });

  it('claude: the Default row names the level and stays the checked row', async () => {
    renderControls({
      provider: 'claude',
      snapshot: snapshot({ settings: { defaultEffort: 'high' } }),
    });
    fireEvent.click(screen.getByText('High'));
    // Both exist: the inherited row and the explicit pin. The ✓ is on Default.
    const marked = (await screen.findAllByText('Default · High')).find(
      (el) => el.tagName === 'SPAN' && el.querySelector('svg'),
    );
    expect(marked).toBeTruthy();
  });

  it('an explicitly pinned effort still shows the pinned level, not the default', () => {
    renderControls({
      provider: 'claude',
      snapshot: snapshot({ settings: { effort: 'low', defaultEffort: 'high' } }),
    });
    expect(screen.getByText('Low')).toBeInTheDocument();
  });

  it('falls back to the bare word when nothing pins a level', () => {
    renderControls({ provider: 'claude', snapshot: snapshot({ settings: {} }) });
    expect(screen.getByText('Default')).toBeInTheDocument();
  });

  it("codex: uses the current model's reported defaultReasoningEffort", async () => {
    api.providerListModels = vi.fn().mockResolvedValue([
      {
        id: 'gpt-5.6-sol',
        label: 'GPT-5.6-Sol',
        default: true,
        effortLevels: ['low', 'medium', 'high', 'xhigh'],
        defaultEffort: 'medium',
      },
    ]);
    renderControls({
      provider: 'codex',
      snapshot: snapshot({ settings: { model: 'gpt-5.6-sol' } }),
    });
    // The catalog is fetched when the menu opens; before that the pill can only
    // say "Default" — codex reports the level per model, not at spawn.
    fireEvent.click(screen.getByText('Default'));
    await waitFor(() => expect(api.providerListModels).toHaveBeenCalled());
    expect(await screen.findByText('Default · Medium')).toBeInTheDocument();
  });
});

describe('ComposerControls — a restart keeps the settings it was not about', () => {
  it('carries the current permission mode through an effort-clearing restart', async () => {
    const { onRestartWith } = renderControls({
      provider: 'claude',
      snapshot: snapshot({ settings: { effort: 'low', permissionMode: 'bypassPermissions' } }),
    });
    fireEvent.click(screen.getByText('Low'));
    fireEvent.click(await screen.findByText('Default'));
    fireEvent.click(await screen.findByText(/Restart with Default effort/));
    // Without this the spawn resolver reads the absent mode as 'default' and the
    // session comes back out of Full access with nothing saying so. The effort
    // stays cleared — that IS what this restart was about.
    expect(onRestartWith).toHaveBeenCalledWith({
      effort: '',
      permissionMode: 'bypassPermissions',
    });
  });

  it('carries a live-switched effort through a permission restart', async () => {
    const { onRestartWith } = renderControls({
      provider: 'claude',
      snapshot: snapshot({
        settings: { permissionMode: 'default', bypassAvailable: false },
        // Set by a live `/effort` switch, not by spawn.
        liveEffort: 'xhigh',
      }),
    });
    fireEvent.click(screen.getByText('Ask to approve'));
    fireEvent.click(await screen.findByText('Full access'));
    fireEvent.click(await screen.findByText(/Restart with Full access/));
    expect(onRestartWith).toHaveBeenCalledWith({
      permissionMode: 'bypassPermissions',
      effort: 'xhigh',
    });
  });

  it('never carries values that are not valid launch argv', async () => {
    const { onRestartWith } = renderControls({
      provider: 'claude',
      snapshot: snapshot({
        settings: { bypassAvailable: false },
        // Both appear in live telemetry but neither is accepted by the launch
        // flags: 'dontAsk' isn't a --permission-mode, 'ultracode' isn't --effort.
        livePermissionMode: 'dontAsk',
        liveEffort: 'ultracode',
      }),
    });
    fireEvent.click(screen.getByText("Don't ask"));
    fireEvent.click(await screen.findByText('Full access'));
    fireEvent.click(await screen.findByText(/Restart with Full access/));
    expect(onRestartWith).toHaveBeenCalledWith({ permissionMode: 'bypassPermissions' });
  });
});

/**
 * Live effort, both providers. Claude takes `/effort <level>` through the message
 * path; codex goes structural. Neither restarts, and a refusal degrades to the
 * restart confirm exactly like the model and permission pills.
 */
describe('ComposerControls — live effort switching', () => {
  it('claude: switching a level calls the live endpoint, not a restart', async () => {
    const { onRestartWith } = renderControls({
      provider: 'claude',
      snapshot: snapshot({ settings: { effort: 'low' } }),
    });
    fireEvent.click(screen.getByText('Low'));
    fireEvent.click(await screen.findByText('Max'));
    expect(api.claudeSetEffort).toHaveBeenCalledWith('sess-1', 'max');
    expect(onRestartWith).not.toHaveBeenCalled();
  });

  it('a refused live switch falls back to the restart confirm with the reason', async () => {
    api.claudeSetEffort = vi
      .fn()
      .mockResolvedValue({ ok: false, error: "this session can't take input right now (stopped)" });
    const { onRestartWith } = renderControls({
      provider: 'claude',
      snapshot: snapshot({ settings: { effort: 'low' } }),
    });
    fireEvent.click(screen.getByText('Low'));
    fireEvent.click(await screen.findByText('High'));
    expect(await screen.findByText(/can't take input right now/)).toBeInTheDocument();
    fireEvent.click(await screen.findByText(/Restart with High effort/));
    await waitFor(() => expect(onRestartWith).toHaveBeenCalledWith({ effort: 'high' }));
  });

  it("the pill prefers the provider's confirmation over what we asked for", () => {
    renderControls({
      provider: 'codex',
      snapshot: snapshot({
        settings: { effort: 'low' },
        liveEffort: 'high',
        // Codex confirmed something else — e.g. the user changed it in its TUI.
        statusLine: { effort: 'xhigh' } as any,
      }),
    });
    expect(screen.getByText('Extra high')).toBeInTheDocument();
  });

  it('the pill prefers a live switch over the spawn-frozen setting', () => {
    renderControls({
      provider: 'claude',
      snapshot: snapshot({ settings: { effort: 'low' }, liveEffort: 'max' }),
    });
    expect(screen.getByText('Max')).toBeInTheDocument();
    expect(screen.queryByText('Low')).not.toBeInTheDocument();
  });
});

describe('ComposerControls — managed provider (codex)', () => {
  it('switching a codex model goes through the managed setModel endpoint', async () => {
    renderControls({
      provider: 'codex',
      snapshot: snapshot({ settings: { model: 'gpt-5-codex' } }),
    });
    fireEvent.click(screen.getByText('gpt-5-codex'));
    const o3 = await screen.findByText('o3');
    fireEvent.click(o3);
    expect(api.claudeSetModel).toHaveBeenCalledWith('sess-1', 'o3');
    expect(api.claudeMessage).not.toHaveBeenCalled();
  });

  it('picking an effort level switches live, no restart', async () => {
    const { onRestartWith } = renderControls({
      provider: 'codex',
      snapshot: snapshot({ settings: { effort: 'low' } }),
    });
    // The effort pill shows the current level.
    fireEvent.click(screen.getByText('Low'));
    fireEvent.click(await screen.findByText('High'));
    expect(api.claudeSetEffort).toHaveBeenCalledWith('sess-1', 'high');
    expect(onRestartWith).not.toHaveBeenCalled();
  });

  it('clearing the override back to the harness default still restarts', async () => {
    const { onRestartWith } = renderControls({
      provider: 'codex',
      snapshot: snapshot({ settings: { effort: 'low' } }),
    });
    fireEvent.click(screen.getByText('Low'));
    fireEvent.click(await screen.findByText('Default'));
    // A live switch can only *set* a level; un-pinning needs a relaunch.
    expect(await screen.findByText(/needs a relaunch/)).toBeInTheDocument();
    fireEvent.click(await screen.findByText(/Restart with Default effort/));
    expect(onRestartWith).toHaveBeenCalledWith({ effort: '' });
    expect(api.claudeSetEffort).not.toHaveBeenCalled();
  });

  it("uses the current Codex model's reported effort names", async () => {
    renderControls({
      provider: 'codex',
      snapshot: snapshot({ settings: { model: 'o3', effort: 'low' } }),
    });
    fireEvent.click(screen.getByText('Low'));

    await waitFor(() => expect(api.providerListModels).toHaveBeenCalled());
    expect(await screen.findByText('High')).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByText('Medium')).not.toBeInTheDocument());
    expect(screen.queryByText('Extra high')).not.toBeInTheDocument();
  });

  it('a failed live model switch falls back to the restart confirm carrying the daemon reason', async () => {
    api.claudeSetModel = vi
      .fn()
      .mockResolvedValue({ ok: false, error: 'rollout fallback can’t switch live' });
    const { onRestartWith } = renderControls({
      provider: 'codex',
      snapshot: snapshot({ settings: { model: 'gpt-5-codex' } }),
    });
    fireEvent.click(screen.getByText('gpt-5-codex'));
    fireEvent.click(await screen.findByText('o3'));
    // The daemon's reason shows in the fallback confirm.
    expect(await screen.findByText(/rollout fallback/)).toBeInTheDocument();
    fireEvent.click(await screen.findByText(/Restart with o3/));
    await waitFor(() => expect(onRestartWith).toHaveBeenCalledWith({ model: 'o3' }));
  });
});

/**
 * A live switch that REJECTS — the promise never resolves at all — is a
 * different failure from a daemon that answers `{ok:false}`, and it used to be
 * the invisible one: every catch was a bare `console.warn`, the pill snapped
 * back to the old label, and the user was left believing the agent had moved
 * to the mode they picked. On a headless hub that is the NORMAL case, not an
 * edge case: `claude.setPermissionMode` / `setModel` / `setEffort` are
 * registered by the desktop main process only, so `/app` against
 * `workspacer serve` gets "no provider for …" every single time. Someone
 * tightening a remote worker's permission mode has to be told it did not take.
 *
 * The fix is the degradation this component already has for a refusal: reopen
 * the menu as the restart confirm carrying the reason — restarting DOES work
 * headless — and post to the notification center so the failure outlives the
 * menu.
 */
describe('ComposerControls — a rejected live switch fails loudly', () => {
  const posted: any[] = [];
  const capture = (e: Event) => posted.push((e as CustomEvent).detail);
  beforeEach(() => {
    posted.length = 0;
    window.addEventListener('wks:notify-post', capture);
  });
  afterEach(() => window.removeEventListener('wks:notify-post', capture));

  it('permission mode: shows the reason instead of writing it to the console', async () => {
    api.claudeSetPermissionMode = vi
      .fn()
      .mockRejectedValue(new Error('no provider for claude.setPermissionMode'));
    const { onRestartWith } = renderControls({
      provider: 'claude',
      snapshot: snapshot({ settings: { permissionMode: 'default' } }),
    });
    fireEvent.click(screen.getByText('Ask to approve'));
    fireEvent.click(await screen.findByText('Plan mode'));

    expect(
      await screen.findByText(/no provider for claude\.setPermissionMode/),
      'the rejection must reach the screen',
    ).toBeInTheDocument();
    // …and the way forward that DOES work headless is offered.
    fireEvent.click(await screen.findByText(/Restart with Plan mode/));
    expect(onRestartWith).toHaveBeenCalledWith(expect.objectContaining({ permissionMode: 'plan' }));
  });

  it('permission mode: posts to the notification center so it outlives the menu', async () => {
    api.claudeSetPermissionMode = vi.fn().mockRejectedValue(new Error('bus is not connected'));
    renderControls({
      provider: 'claude',
      snapshot: snapshot({ settings: { permissionMode: 'default' } }),
    });
    fireEvent.click(screen.getByText('Ask to approve'));
    fireEvent.click(await screen.findByText('Plan mode'));

    await waitFor(() => expect(posted.length).toBe(1));
    expect(posted[0].level).toBe('error');
    expect(String(posted[0].title)).toMatch(/permission mode/i);
    expect(String(posted[0].body)).toMatch(/bus is not connected/);
  });

  it('permission mode: the pill never keeps showing the mode it failed to reach', async () => {
    api.claudeSetPermissionMode = vi.fn().mockRejectedValue(new Error('no provider'));
    renderControls({
      provider: 'claude',
      snapshot: snapshot({ settings: { permissionMode: 'default' } }),
    });
    fireEvent.click(screen.getByText('Ask to approve'));
    fireEvent.click(await screen.findByText('Plan mode'));
    await waitFor(() => expect(screen.queryByText('Plan mode…')).not.toBeInTheDocument());
  });

  it('effort: a rejection degrades to the restart confirm with the reason', async () => {
    api.claudeSetEffort = vi.fn().mockRejectedValue(new Error('no provider for claude.setEffort'));
    const { onRestartWith } = renderControls({
      provider: 'claude',
      snapshot: snapshot({ settings: { effort: 'low' } }),
    });
    fireEvent.click(screen.getByText('Low'));
    fireEvent.click(await screen.findByText('High'));

    expect(await screen.findByText(/no provider for claude\.setEffort/)).toBeInTheDocument();
    fireEvent.click(await screen.findByText(/Restart with High effort/));
    expect(onRestartWith).toHaveBeenCalledWith(expect.objectContaining({ effort: 'high' }));
  });

  it('model (managed provider): a rejection degrades to the restart confirm', async () => {
    api.claudeSetModel = vi.fn().mockRejectedValue(new Error('no provider for claude.setModel'));
    const { onRestartWith } = renderControls({
      provider: 'codex',
      snapshot: snapshot({ settings: { model: 'gpt-5-codex' } }),
    });
    fireEvent.click(screen.getByText('gpt-5-codex'));
    fireEvent.click(await screen.findByText('o3'));

    expect(await screen.findByText(/no provider for claude\.setModel/)).toBeInTheDocument();
    fireEvent.click(await screen.findByText(/Restart with o3/));
    expect(onRestartWith).toHaveBeenCalledWith(expect.objectContaining({ model: 'o3' }));
  });

  it('model (claude slash-command path): a rejected send is not swallowed either', async () => {
    api.claudeMessage = vi
      .fn()
      .mockRejectedValue(new Error('hub call timeout: agents.sendMessage'));
    const { onRestartWith } = renderControls({
      provider: 'claude',
      snapshot: snapshot({ settings: { model: 'sonnet' } }),
    });
    fireEvent.click(screen.getByText('sonnet'));
    fireEvent.click(await screen.findByText('Opus'));

    expect(await screen.findByText(/hub call timeout/)).toBeInTheDocument();
    fireEvent.click(await screen.findByText(/Restart with Opus/));
    expect(onRestartWith).toHaveBeenCalledWith(expect.objectContaining({ model: 'opus' }));
  });
});
