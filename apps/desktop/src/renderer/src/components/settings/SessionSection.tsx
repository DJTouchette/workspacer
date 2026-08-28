import React, { useEffect, useState } from 'react';
import { Config } from '../../hooks/useConfig';
import { Check, X } from 'lucide-react';
import {
  PERMISSION_MODE_DEFAULTS,
  currentPermissionModeDefault,
  permissionModeDefaultPatch,
} from '../../lib/permissionDefaults';
import {
  Section,
  CheckRow,
  Row,
  ModeButton,
  SearchableSelect,
  type SelectOption,
} from './primitives';
import HarnessModelSelect from './HarnessModelSelect';
import { isForeignModel } from '../../../../main/shared/modelVocabulary';
import type { AgentProvider } from '../../types/pane';

interface SessionSectionProps {
  config: Config;
  save: (partial: Partial<Config>) => Promise<Config>;
}

/** Harnesses that can answer a one-shot title call — every provider with a
 *  `directCompletion` adapter (services/directCompletion). One row each,
 *  because the titler uses the AGENT's harness, not a configured one. */
const TITLE_PROVIDERS: { label: string; value: AgentProvider }[] = [
  { label: 'Claude', value: 'claude' },
  { label: 'Codex', value: 'codex' },
  { label: 'OpenCode', value: 'opencode' },
  { label: 'Pi', value: 'pi' },
];

const AGENT_PROVIDERS: {
  label: string;
  value: 'claude' | 'codex' | 'copilot' | 'opencode' | 'pi';
  beta?: boolean;
}[] = [
  { label: 'Claude', value: 'claude' },
  { label: 'Codex', value: 'codex' },
  // Not yet thoroughly tested — flagged Beta so expectations are set.
  { label: 'GitHub Copilot', value: 'copilot', beta: true },
  { label: 'OpenCode', value: 'opencode', beta: true },
  { label: 'Pi', value: 'pi', beta: true },
];

/** Provider label with a Beta suffix for not-yet-hardened backends. */
const providerLabel = (p: { label: string; beta?: boolean }): string =>
  p.beta ? `${p.label} (Beta)` : p.label;

interface ProviderDetection {
  provider: string;
  found: boolean;
  resolvedPath: string | null;
  customBin: string;
}

/** One editable binary-path row for a provider in the settings panel. */
const BinaryRow: React.FC<{
  label: string;
  providerId: 'claude' | 'codex' | 'copilot' | 'opencode' | 'pi';
  detection: ProviderDetection | undefined;
  config: Config;
  save: (partial: Partial<Config>) => Promise<Config>;
  onRefresh: () => void;
}> = ({ label, providerId, detection, config, save, onRefresh }) => {
  const [value, setValue] = useState(config.agents?.binaries?.[providerId] ?? '');
  useEffect(() => {
    setValue(config.agents?.binaries?.[providerId] ?? '');
  }, [config.agents?.binaries, providerId]);

  const persist = (v: string) => {
    save({
      agents: {
        ...config.agents,
        binaries: { ...config.agents?.binaries, [providerId]: v.trim() },
      },
    })
      .then(() => onRefresh())
      .catch(() => {});
  };

  const browse = async () => {
    const files = await window.electronAPI.pickFiles?.(undefined);
    if (files?.length) {
      setValue(files[0]);
      persist(files[0]);
    }
  };

  const dotColor =
    detection === undefined
      ? 'var(--wks-text-disabled)'
      : detection.found
        ? '#3db86a'
        : 'var(--wks-error)';
  const hint =
    detection === undefined
      ? 'Checking…'
      : detection.found
        ? `Found: ${detection.resolvedPath}`
        : 'Not found on PATH';

  return (
    <Row label={label}>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 4, flex: 1, minWidth: 0 }}>
        <div style={{ display: 'flex', gap: 4 }}>
          <input
            value={value}
            onChange={(e) => setValue(e.target.value)}
            onBlur={(e) => persist(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') persist(value);
            }}
            placeholder="Auto-detect on PATH"
            spellCheck={false}
            style={{
              flex: 1,
              minWidth: 0,
              fontSize: '0.7rem',
              fontFamily: 'inherit',
              background: 'var(--wks-bg-base)',
              color: 'var(--wks-text-primary)',
              border: '1px solid var(--wks-border-input)',
              borderRadius: 4,
              padding: '4px 7px',
            }}
          />
          <button
            onClick={browse}
            style={{
              fontSize: '0.7rem',
              fontFamily: 'inherit',
              cursor: 'pointer',
              whiteSpace: 'nowrap',
              background: 'var(--wks-bg-input)',
              color: 'var(--wks-text-tertiary)',
              border: '1px solid var(--wks-border-input)',
              borderRadius: 4,
              padding: '0 10px',
            }}
          >
            Browse…
          </button>
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 5, fontSize: '0.64rem' }}>
          <span
            style={{
              width: 7,
              height: 7,
              borderRadius: '50%',
              background: dotColor,
              flexShrink: 0,
            }}
          />
          <span style={{ color: detection?.found ? '#3db86a' : 'var(--wks-text-disabled)' }}>
            {hint}
          </span>
        </div>
      </div>
    </Row>
  );
};

const SessionSection: React.FC<SessionSectionProps> = ({ config, save }) => {
  const defaultView = config.claude?.defaultView ?? 'terminal';
  // Fallback mirrors config_defaults.json — the shipped default transport is
  // 'stream', so an absent key must not render as PTY.
  const claudeTransport = config.claude?.transport ?? 'stream';
  // Codex's twin. Ships 'stream' too — the headless app-server path is the one
  // that mirrors Claude's stream transport, and the hybrid stays one click away.
  const codexTransport = config.codex?.transport ?? 'stream';
  const defaultProvider = config.agents?.defaultProvider ?? 'claude';
  const keepWarm = {
    enabled: config.claude?.keepWarm?.enabled ?? false,
    providers: config.claude?.keepWarm?.providers ?? ['claude'],
    mode: config.claude?.keepWarm?.mode ?? ('auto' as const),
    intervalHours: config.claude?.keepWarm?.intervalHours ?? 5,
    dailyAt: config.claude?.keepWarm?.dailyAt ?? '08:00',
  };
  const saveKeepWarm = (patch: Partial<typeof keepWarm>) =>
    save({ claude: { ...config.claude, defaultView, keepWarm: { ...keepWarm, ...patch } } });

  // The spawn-time permission default. Two config keys, one control — both are
  // written together so they can never contradict (see lib/permissionDefaults).
  const permissionMode = currentPermissionModeDefault(config.claude);
  const savePermissionMode = (mode: string) =>
    save({
      claude: { ...config.claude, defaultView, ...permissionModeDefaultPatch(mode) },
    });

  // Recent keep-warm heartbeats from claudemon's log (the "warms" list).
  const [heartbeats, setHeartbeats] = useState<
    Array<{
      id: number;
      at: number;
      ok: boolean;
      provider?: string;
      resets_at: number | null;
      error: string | null;
    }>
  >([]);
  useEffect(() => {
    if (!keepWarm.enabled) return;
    let cancelled = false;
    window.electronAPI
      .keepWarmHeartbeats?.(8)
      .then((rows) => {
        if (!cancelled) setHeartbeats(rows ?? []);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [keepWarm.enabled]);

  const [detection, setDetection] = useState<ProviderDetection[]>([]);
  const refreshDetection = () => {
    window.electronAPI
      .providerCheckAll?.()
      .then((list) => setDetection(list ?? []))
      .catch(() => {});
  };
  useEffect(() => {
    refreshDetection();
  }, []);

  // Default directory for new agents. Local state so typing is smooth; persisted
  // on blur / Enter (and immediately when picked via Browse).
  const [defaultCwd, setDefaultCwd] = React.useState(config.agents?.defaultCwd ?? '');
  React.useEffect(() => {
    setDefaultCwd(config.agents?.defaultCwd ?? '');
  }, [config.agents?.defaultCwd]);
  const saveDefaultCwd = (value: string) => {
    const v = value.trim();
    if (v === (config.agents?.defaultCwd ?? '')) return;
    save({ agents: { ...config.agents, defaultCwd: v } });
  };
  // Worktree root: local state for smooth typing, persisted on blur/Enter.
  const [worktreeRoot, setWorktreeRoot] = React.useState(config.agents?.worktreeRoot ?? '');
  // Which harness's title model is being edited. Auto-titling is not tied to
  // ONE harness the way the supervisor is — every agent is titled by its own —
  // so this selects a row to edit rather than choosing a backend, and all of
  // them stay live at once in `autoTitle.models`.
  const [titleHarness, setTitleHarness] = useState<AgentProvider>(
    config.agents?.defaultProvider ?? 'claude',
  );
  const autoTitle = config.agents?.autoTitle;
  // Same resolution main uses (lib/roleModels): this harness's entry, then the
  // legacy single field but only where it is servable — `'haiku'` is a claude
  // alias and must not be shown as codex's configured title model.
  const titleModel =
    autoTitle?.models?.[titleHarness] ??
    (isForeignModel(titleHarness, autoTitle?.model) ? '' : (autoTitle?.model ?? ''));
  const setTitleModel = (v: string) =>
    save({
      agents: {
        ...config.agents,
        autoTitle: {
          ...autoTitle,
          models: { ...(autoTitle?.models ?? {}), [titleHarness]: v },
          // Keep the legacy field in step only while it means the same thing on
          // this harness, so an old config and a new one never disagree.
          ...(!isForeignModel(titleHarness, v) && { model: v }),
        },
      },
    });
  React.useEffect(() => {
    setWorktreeRoot(config.agents?.worktreeRoot ?? '');
  }, [config.agents?.worktreeRoot]);
  const saveWorktreeRoot = (value: string) => {
    const v = value.trim();
    if (v === (config.agents?.worktreeRoot ?? '')) return;
    save({ agents: { ...config.agents, worktreeRoot: v } });
  };

  const browseDefaultCwd = async () => {
    const picked = await window.electronAPI.pickFolder?.(defaultCwd || undefined);
    if (picked) {
      setDefaultCwd(picked);
      saveDefaultCwd(picked);
    }
  };

  return (
    <Section title="Session">
      <Row label="Default agent">
        <div style={{ display: 'flex', gap: 4 }}>
          {AGENT_PROVIDERS.map((p) => (
            <ModeButton
              key={p.value}
              label={providerLabel(p)}
              active={defaultProvider === p.value}
              onClick={() => save({ agents: { ...config.agents, defaultProvider: p.value } })}
            />
          ))}
        </div>
      </Row>
      <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-disabled)' }}>
        The coding agent pre-selected in the spawn dialog. Codex and OpenCode run via claudemon's
        adapters with live telemetry; Claude is the default.
      </div>

      <Row label="Default directory">
        <div style={{ display: 'flex', gap: 4, flex: 1, minWidth: 0 }}>
          <input
            value={defaultCwd}
            onChange={(e) => setDefaultCwd(e.target.value)}
            onBlur={(e) => saveDefaultCwd(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') saveDefaultCwd((e.target as HTMLInputElement).value);
            }}
            placeholder="App launch directory"
            spellCheck={false}
            style={{
              flex: 1,
              minWidth: 0,
              fontSize: '0.7rem',
              fontFamily: 'inherit',
              background: 'var(--wks-bg-base)',
              color: 'var(--wks-text-primary)',
              border: '1px solid var(--wks-border-input)',
              borderRadius: 4,
              padding: '4px 7px',
            }}
          />
          <button
            onClick={browseDefaultCwd}
            style={{
              fontSize: '0.7rem',
              fontFamily: 'inherit',
              cursor: 'pointer',
              whiteSpace: 'nowrap',
              background: 'var(--wks-bg-input)',
              color: 'var(--wks-text-tertiary)',
              border: '1px solid var(--wks-border-input)',
              borderRadius: 4,
              padding: '0 10px',
            }}
          >
            Browse…
          </button>
        </div>
      </Row>
      <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-disabled)' }}>
        Where the spawn dialog opens (and where Browse… starts). Leave blank to use the app's launch
        directory.
      </div>

      <CheckRow
        label="Start new agents in a git worktree"
        checked={config.agents?.spawnInWorktree ?? false}
        onChange={(v) => save({ agents: { ...config.agents, spawnInWorktree: v } })}
      />
      <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-disabled)' }}>
        Pre-checks "isolated worktree" in the spawn dialog: each agent gets a fresh git worktree on
        its own branch, so parallel agents in one repo never collide and everything scoped to the
        agent (plugins, watchers, checks) is confined to its tree. Worktrees persist until you
        remove them (<code>git worktree remove</code>) — they may hold uncommitted work.
      </div>

      <Row label="Worktree location">
        <input
          value={worktreeRoot}
          onChange={(e) => setWorktreeRoot(e.target.value)}
          onBlur={(e) => saveWorktreeRoot(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') saveWorktreeRoot((e.target as HTMLInputElement).value);
          }}
          placeholder="~/.workspacer/worktrees"
          spellCheck={false}
          style={{
            flex: 1,
            minWidth: 0,
            fontSize: '0.7rem',
            fontFamily: 'inherit',
            background: 'var(--wks-bg-base)',
            color: 'var(--wks-text-primary)',
            border: '1px solid var(--wks-border-input)',
            borderRadius: 4,
            padding: '4px 7px',
          }}
        />
      </Row>
      <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-disabled)' }}>
        Parent directory for agent worktrees (created as &lt;repo&gt;/&lt;agent&gt; inside it).
        Leave blank for the default.
      </div>

      <CheckRow
        label="Name agents after their first exchange"
        checked={config.agents?.autoTitle?.enabled !== false}
        onChange={(v) =>
          save({
            agents: { ...config.agents, autoTitle: { ...config.agents?.autoTitle, enabled: v } },
          })
        }
      />
      <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-disabled)' }}>
        Once an agent has answered its first message, a short title replaces the folder name on its
        card and tab — the way a chat service names a conversation. One cheap model call per agent,
        never repeated. A name you type yourself is never overwritten, and if the call can't run,
        the agent falls back to the first line of what you asked.
      </div>

      <Row label="Title model for">
        <div style={{ display: 'flex', gap: 4 }}>
          {TITLE_PROVIDERS.map((p) => (
            <ModeButton
              key={p.value}
              label={p.label}
              active={titleHarness === p.value}
              onClick={() => setTitleHarness(p.value)}
            />
          ))}
        </div>
      </Row>
      <HarnessModelSelect
        provider={titleHarness}
        label={`${TITLE_PROVIDERS.find((p) => p.value === titleHarness)?.label} title model`}
        value={titleModel}
        onChange={setTitleModel}
        defaultLabel={`${titleHarness} default`}
        hint={
          <>
            Writing a title is a trivial task — keep this cheap. The call runs on the harness{' '}
            <strong>the agent itself uses</strong>, not a fixed one, so each harness gets its own
            choice and a mixed fleet doesn’t have to share a model none of them agree on. Leave it
            on the default to let that CLI pick.
          </>
        }
        warningSuffix="Titling falls back to the first line of your own message when the call can’t run."
      />

      <Row label="Claude transport">
        <div style={{ display: 'flex', gap: 4 }}>
          <ModeButton
            label="Terminal (PTY)"
            active={claudeTransport === 'pty'}
            onClick={() => save({ claude: { ...config.claude, defaultView, transport: 'pty' } })}
          />
          <ModeButton
            label="Headless (stream)"
            active={claudeTransport === 'stream'}
            onClick={() => save({ claude: { ...config.claude, defaultView, transport: 'stream' } })}
          />
        </div>
      </Row>
      <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-disabled)' }}>
        How new Claude sessions run. Terminal (PTY) is the classic Claude Code TUI with both Term
        and GUI views. Headless (stream) runs Claude via claudemon's stream-json adapter — the
        structured GUI only, no terminal view. Overridable per spawn in the spawn dialog.
      </div>

      <Row label="Codex transport">
        <div style={{ display: 'flex', gap: 4 }}>
          <ModeButton
            label="Hybrid (TUI + GUI)"
            active={codexTransport === 'pty'}
            onClick={() => save({ codex: { ...config.codex, transport: 'pty' } })}
          />
          <ModeButton
            label="Headless (stream)"
            active={codexTransport === 'stream'}
            onClick={() => save({ codex: { ...config.codex, transport: 'stream' } })}
          />
        </div>
      </Row>
      <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-disabled)' }}>
        How new Codex sessions run. Headless (stream) is the default: claudemon drives{' '}
        <code>codex app-server</code> over a websocket and the structured GUI is the only surface —
        the exact twin of Claude's headless transport. Hybrid also runs the native Codex TUI in a
        terminal, rejoined onto the same thread, so you get a Term view too. Overridable per spawn.
      </div>

      <Row label="Default permission mode">
        <div style={{ display: 'flex', gap: 4, flexWrap: 'wrap' }}>
          {PERMISSION_MODE_DEFAULTS.map((m) => (
            <ModeButton
              key={m.value || 'default'}
              label={m.label}
              active={permissionMode === m.value}
              onClick={() => savePermissionMode(m.value)}
            />
          ))}
        </div>
      </Row>
      <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-disabled)' }}>
        What a NEW agent starts in — the spawn dialog pre-selects it, and agents dispatched by the
        Fleet Manager or the supervisor inherit it when they do not ask for a mode themselves.{' '}
        <strong>Full access</strong> means no per-action approval prompts at all: fast and
        hands-off, with no human gate on each command. Applies to newly spawned sessions only —
        running agents keep the mode they were started with (switch a live one from its pane’s
        permission pill).
      </div>

      <CheckRow
        label="Keep hooks out of my global Claude settings (experimental)"
        checked={config.claude?.settingsOverlay === true}
        onChange={(v) => save({ claude: { ...config.claude, defaultView, settingsOverlay: v } })}
      />
      <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-disabled)' }}>
        Installs claudemon's hooks and status-line into a private overlay file passed to Claude via
        --settings, instead of writing them into ~/.claude/settings.json. Applies to new sessions;
        toggling on also removes any previously-installed entries from your global file. Restart the
        app after changing this.
      </div>

      <Row label="Default Claude view">
        <div style={{ display: 'flex', gap: 4 }}>
          <ModeButton
            label="GUI"
            active={defaultView === 'gui'}
            onClick={() => save({ claude: { ...config.claude, defaultView: 'gui' } })}
          />
          <ModeButton
            label="Terminal"
            active={defaultView === 'terminal'}
            onClick={() => save({ claude: { ...config.claude, defaultView: 'terminal' } })}
          />
        </div>
      </Row>
      <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-disabled)' }}>
        Which view a Claude pane opens in. The rich GUI shows the conversation, work cards, and
        inspector; Terminal is the raw Claude Code TUI. Toggle any time from the pane's top bar.
      </div>

      <CheckRow
        label="Keep 5-hour window warm"
        checked={keepWarm.enabled}
        onChange={(v) => saveKeepWarm({ enabled: v })}
      />
      <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-disabled)' }}>
        Subscription usage runs in 5-hour windows that only start with your first message. When the
        current window is expired (0%), this sends one minimal Haiku ping so a fresh window is
        already running before you sit down. Checks account usage first — never pings mid-window.
        Runs only while Workspacer is open.
      </div>
      {keepWarm.enabled && (
        <>
          <Row label="Warm providers">
            <div style={{ display: 'flex', gap: 4 }}>
              {(['claude', 'codex'] as const).map((p) => (
                <ModeButton
                  key={p}
                  label={p === 'claude' ? 'Claude' : 'Codex'}
                  active={keepWarm.providers.includes(p)}
                  onClick={() =>
                    saveKeepWarm({
                      providers: keepWarm.providers.includes(p)
                        ? keepWarm.providers.filter((x) => x !== p)
                        : [...keepWarm.providers, p],
                    })
                  }
                />
              ))}
            </div>
          </Row>
          <Row label="Warm trigger">
            <div style={{ display: 'flex', gap: 4 }}>
              <ModeButton
                label="Always"
                active={keepWarm.mode === 'auto'}
                onClick={() => saveKeepWarm({ mode: 'auto' })}
              />
              <ModeButton
                label="Every N hours"
                active={keepWarm.mode === 'interval'}
                onClick={() => saveKeepWarm({ mode: 'interval' })}
              />
              <ModeButton
                label="Daily at"
                active={keepWarm.mode === 'daily'}
                onClick={() => saveKeepWarm({ mode: 'daily' })}
              />
            </div>
          </Row>
          {keepWarm.mode === 'interval' && (
            <Row label="Check every (hours)">
              <input
                type="number"
                min={1}
                max={24}
                value={keepWarm.intervalHours}
                onChange={(e) => {
                  const n = Math.max(1, Math.min(24, Number(e.target.value) || 5));
                  void saveKeepWarm({ intervalHours: n });
                }}
                style={{
                  width: 64,
                  fontSize: '0.78rem',
                  fontFamily: 'inherit',
                  color: 'var(--wks-text-primary)',
                  background: 'var(--wks-bg-input)',
                  border: '1px solid var(--wks-border-input)',
                  borderRadius: 'var(--wks-radius-sm)',
                  padding: '4px 7px',
                }}
              />
            </Row>
          )}
          {keepWarm.mode === 'daily' && (
            <Row label="Check daily at">
              <input
                type="time"
                value={keepWarm.dailyAt}
                onChange={(e) => {
                  if (e.target.value) void saveKeepWarm({ dailyAt: e.target.value });
                }}
                style={{
                  fontSize: '0.78rem',
                  fontFamily: 'inherit',
                  color: 'var(--wks-text-primary)',
                  background: 'var(--wks-bg-input)',
                  border: '1px solid var(--wks-border-input)',
                  borderRadius: 'var(--wks-radius-sm)',
                  padding: '4px 7px',
                }}
              />
            </Row>
          )}
          <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-disabled)' }}>
            {keepWarm.mode === 'auto'
              ? 'Re-warms the moment a window lapses, so a 5-hour window is always running.'
              : keepWarm.mode === 'interval'
                ? 'Checks on this cadence and warms only if no window is running at check time.'
                : 'Checks once a day at this local time and warms only if no window is running.'}
          </div>
          {heartbeats.length > 0 && (
            <div
              style={{
                display: 'flex',
                flexDirection: 'column',
                gap: 3,
                padding: '8px 10px',
                background: 'var(--wks-bg-raised)',
                border: '1px solid var(--wks-border-subtle)',
                borderRadius: 'var(--wks-radius-md)',
              }}
            >
              <div
                style={{
                  fontSize: '0.62rem',
                  fontWeight: 600,
                  textTransform: 'uppercase',
                  letterSpacing: '0.08em',
                  color: 'var(--wks-text-faint)',
                  marginBottom: 2,
                }}
              >
                Recent warms
              </div>
              {heartbeats.map((h) => (
                <div
                  key={h.id}
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 6,
                    fontSize: '0.7rem',
                    fontFamily: 'var(--wks-font-mono)',
                    color: 'var(--wks-text-muted)',
                  }}
                >
                  {h.ok ? (
                    <Check size={11} strokeWidth={2.25} style={{ color: 'var(--wks-success)' }} />
                  ) : (
                    <X size={11} strokeWidth={2.25} style={{ color: 'var(--wks-error)' }} />
                  )}
                  <span>{new Date(h.at * 1000).toLocaleString()}</span>
                  {h.provider && (
                    <span style={{ color: 'var(--wks-text-tertiary)' }}>{h.provider}</span>
                  )}
                  <span style={{ color: 'var(--wks-text-faint)' }}>
                    {h.ok
                      ? h.resets_at != null
                        ? `window until ${new Date(h.resets_at * 1000).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })}`
                        : 'window started'
                      : (h.error ?? 'failed')}
                  </span>
                </div>
              ))}
            </div>
          )}
        </>
      )}

      <div
        style={{
          fontSize: '0.68rem',
          fontWeight: 600,
          color: 'var(--wks-text-muted)',
          marginTop: 16,
          marginBottom: 4,
        }}
      >
        Tool paths
      </div>
      <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-disabled)', marginBottom: 8 }}>
        Override the binary path for each coding agent. Leave blank to auto-detect on PATH. A green
        dot means the CLI was found; red means it's missing or the path is invalid.
      </div>
      {AGENT_PROVIDERS.map((p) => (
        <BinaryRow
          key={p.value}
          label={providerLabel(p)}
          providerId={p.value}
          detection={detection.find((d) => d.provider === p.value)}
          config={config}
          save={save}
          onRefresh={refreshDetection}
        />
      ))}
    </Section>
  );
};

export default SessionSection;
