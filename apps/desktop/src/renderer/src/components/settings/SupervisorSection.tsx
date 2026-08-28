import React, { useEffect, useState } from 'react';
import { Config } from '../../hooks/useConfig';
import type { AgentProvider } from '../../types/pane';
import { capsFor } from '../../lib/providerCaps';
import { loadModelOptions } from '../../lib/modelOptions';
import {
  Section,
  Row,
  SearchableSelect,
  SelectOption,
  ModeButton,
  CheckRow,
  inputStyle,
} from './primitives';

/**
 * Harnesses the Supervisor ROLE is verified on. Pi is deliberately absent:
 * the supervisor's whole job is watching the fleet through the workspacer MCP
 * facade and notifying you, but pi core ships no MCP client at all — `pi.rs`
 * warns facade tools are unavailable to it, `managedSpawn.ts` refuses to mint
 * it a facade token (`provider !== 'pi'`), and `agentSkillsRoot` returns null
 * for it so it never gets the /supervise skill either. A "Pi supervisor"
 * would run on role instructions alone with no way to observe or coordinate
 * anything — the same failure mode MANAGER_PROVIDERS below already excludes
 * Pi (and OpenCode) to avoid.
 */
const SUP_PROVIDERS: { value: 'claude' | 'codex' | 'opencode'; label: string }[] = [
  { value: 'claude', label: 'Claude' },
  { value: 'codex', label: 'Codex' },
  { value: 'opencode', label: 'OpenCode' },
];

/**
 * Harnesses the Fleet Manager ROLE is verified on. Narrower than SUP_PROVIDERS
 * on purpose: the manager needs an MCP client to dispatch at all, and a
 * personal-skills directory for /standup, /checkpoint and /handoff. Claude and
 * Codex have both (`~/.claude/skills` / `$CODEX_HOME/skills`, identical
 * SKILL.md format); listing a harness that silently loses half the role is the
 * failure mode this picker exists to avoid.
 */
const MANAGER_PROVIDERS: { value: 'claude' | 'codex'; label: string }[] = [
  { value: 'claude', label: 'Claude' },
  { value: 'codex', label: 'Codex' },
];

interface SupervisorSectionProps {
  config: Config;
  save: (partial: Partial<Config>) => Promise<Config>;
}

/**
 * The model dropdown's options FOR ONE HARNESS.
 *
 * The bug this closes: this used to call `claudeListModels()` unconditionally,
 * so picking Codex as the supervisor agent left the picker offering Claude
 * aliases (`fable`, `opus`, …). Saving one of those wrote a model id the codex
 * CLI rejects — the field looked configured and the spawn 400'd.
 *
 * `loadModelOptions` is the same source the spawn dialog and the composer pill
 * read, keyed on the provider's `modelSource`: Claude's curated aliases + the
 * ids seen across sessions, or the daemon's live per-provider catalog (which
 * boots that CLI to ask it). Never throws — a provider that isn't installed or
 * authed resolves to an empty list, which renders as "harness default only".
 */
function useModelOptions(provider: AgentProvider): { options: SelectOption[]; loaded: boolean } {
  const [options, setOptions] = useState<SelectOption[]>([]);
  const [loaded, setLoaded] = useState(false);
  useEffect(() => {
    let cancelled = false;
    setLoaded(false);
    setOptions([]);
    void loadModelOptions(provider, capsFor(provider).modelSource).then((list) => {
      if (cancelled) return;
      setOptions(list.map((m) => ({ value: m.id, label: m.label })));
      setLoaded(true);
    });
    return () => {
      cancelled = true;
    };
  }, [provider]);
  return { options, loaded };
}

const SupervisorSection: React.FC<SupervisorSectionProps> = ({ config, save }) => {
  const sup = config.supervisor ?? {};
  const supProvider: AgentProvider = sup.provider ?? 'claude';
  const model = sup.model ?? '';
  const summarizerModel = sup.summarizerModel ?? 'sonnet';
  const pollSeconds = sup.pollSeconds ?? 45;

  const { options: harnessModels, loaded: harnessModelsLoaded } = useModelOptions(supProvider);
  // The current value is always offered even when the harness's catalog doesn't
  // know it (a hand-edited config, a model since retired) — dropping it from
  // the list would render the field blank while the config still held it. It's
  // flagged below instead.
  const modelOptions: SelectOption[] = [
    { value: '', label: supProvider === 'claude' ? 'App default' : 'Harness default' },
    ...harnessModels,
    ...(model && !harnessModels.some((o) => o.value === model)
      ? [{ value: model, label: `${model} (not in ${supProvider}’s catalog)` }]
      : []),
  ];
  const modelUnknown =
    !!model && harnessModelsLoaded && !harnessModels.some((o) => o.value === model);

  // The summarizer list stays CLAUDE's whatever harness the supervisor runs on:
  // the /supervise skill spawns its digest worker via spawn_agent without
  // naming a provider, and that path spawns Claude. A codex supervisor still
  // dispatches Claude summarizers.
  const { options: claudeModels } = useModelOptions('claude');
  const summarizerOptions: SelectOption[] = claudeModels.filter((o) => o.value !== '');

  const patch = (p: Partial<NonNullable<Config['supervisor']>>) =>
    save({ supervisor: { ...sup, ...p } });

  /**
   * Switch harness. The model field belongs to the harness it was chosen on —
   * `fable` is meaningless to codex and would 400 at spawn — so the outgoing
   * choice is filed under the outgoing provider and the incoming one's
   * remembered choice (or the harness default) takes its place. Flipping back
   * and forth is lossless; nothing is silently left pointing at the wrong CLI.
   */
  const setProvider = (next: AgentProvider) => {
    if (next === supProvider) return;
    const models = { ...(sup.models ?? {}), [supProvider]: model };
    return patch({ provider: next, model: models[next] ?? '', models });
  };

  /** Save the model AND remember it for this harness (see setProvider). */
  const setModel = (v: string) =>
    patch({ model: v, models: { ...(sup.models ?? {}), [supProvider]: v } });

  const agents = config.agents ?? {};
  const fleetRoot = agents.fleetRoot ?? '';
  const fleetFullAccess = agents.fleetFullAccess === true;
  const patchAgents = (p: Partial<NonNullable<Config['agents']>>) =>
    save({ agents: { ...agents, ...p } });

  return (
    <Section title="Supervisor">
      <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-disabled)', marginBottom: 8 }}>
        Optional. The supervisor is the agent started by “Ask the Fleet”. It watches your other
        agents, summarizes what they’re doing, and notifies you when a decision is needed. Nothing
        here runs unless you spawn one.
      </div>

      <Row label="Supervisor agent">
        <div style={{ display: 'flex', gap: 4 }}>
          {SUP_PROVIDERS.map((p) => (
            <ModeButton
              key={p.value}
              label={p.label}
              active={supProvider === p.value}
              onClick={() => setProvider(p.value)}
            />
          ))}
        </div>
      </Row>
      <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-disabled)' }}>
        Which CLI the supervisor runs on (also pickable when you launch one from “Ask the Fleet”).
        Codex and OpenCode supervisors are wired to the workspacer MCP facade — the supervisor’s
        tools to observe and coordinate agents — via their own MCP config. Experimental: it needs a
        CLI build with remote-MCP support; Claude remains the most battle-tested.
      </div>

      <Row label="Supervisor model">
        <SearchableSelect
          value={model}
          options={modelOptions}
          onChange={setModel}
          placeholder={supProvider === 'claude' ? 'App default' : 'Harness default'}
        />
      </Row>
      <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-disabled)' }}>
        The coordinator model, from the models the harness above actually offers. Keep this strong —
        it reasons over the fleet and composes notifications. Each harness remembers its own choice,
        so switching back and forth doesn’t lose it.
      </div>
      {modelUnknown && (
        <div style={{ fontSize: '0.72rem', color: 'var(--wks-warning)' }}>
          <strong>{model}</strong> is not in {supProvider}’s model list — a supervisor started on
          this harness will be refused it. Pick one above, or leave it on the harness default.
        </div>
      )}

      <Row label="Summarizer model">
        <SearchableSelect
          value={summarizerModel}
          options={summarizerOptions}
          onChange={(v) => patch({ summarizerModel: v })}
          placeholder="sonnet"
        />
      </Row>
      <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-disabled)' }}>
        The cheap model the supervisor spawns to read transcripts and write digests. Sonnet by
        default; Haiku is cheaper.
      </div>

      <Row label="Poll interval (seconds)">
        <input
          type="number"
          min={5}
          max={3600}
          value={pollSeconds}
          onChange={(e) => {
            const n = Math.round(Number(e.target.value));
            if (Number.isFinite(n) && n > 0) patch({ pollSeconds: n });
          }}
          style={{ ...inputStyle, width: 80 }}
        />
      </Row>
      <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-disabled)' }}>
        How often the supervisor re-sweeps the fleet for status and pending decisions.
      </div>

      <CheckRow
        label="Full access (supervisor + its workers skip approvals)"
        checked={sup.fullAccess === true}
        onChange={(v) => patch({ fullAccess: v })}
      />
      <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-disabled)' }}>
        When on, the workers the supervisor spawns skip approvals — no per-action approval prompts.
        Hands-off, but there is no human gate on each command. Off by default.{' '}
        <strong>Takes effect for newly spawned sessions.</strong> A running supervisor picks this up
        immediately for agents it spawns from now on, but its OWN tool calls keep the permission
        mode it was started with — a session’s bypass is fixed when it spawns — so respawn it to
        change that.
      </div>

      <div style={{ marginTop: 18, fontWeight: 600, fontSize: '0.8rem' }}>Fleet Manager</div>
      <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-disabled)', margin: '4px 0 8px' }}>
        The delegating manager launched from the Overview. It dispatches real agents into your
        projects and reports back — see “Fleet Manager” on the dashboard.
      </div>

      <Row label="Manager agent">
        <div style={{ display: 'flex', gap: 4 }}>
          {MANAGER_PROVIDERS.map((p) => (
            <ModeButton
              key={p.value}
              label={p.label}
              active={(agents.managerProvider ?? 'claude') === p.value}
              onClick={() => patchAgents({ managerProvider: p.value })}
            />
          ))}
        </div>
      </Row>
      <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-disabled)' }}>
        The harness the manager itself runs on. It dispatches workers on any harness either way —
        this is only which one hosts the manager’s own conversation.{' '}
        <strong>Applies to the next manager you start.</strong> A conversation cannot move between
        harnesses, so an existing Fleet Manager card keeps the one it was started on; terminate it
        (right-click → Terminate) to start a fresh manager here.
      </div>

      <Row label="Projects root">
        <input
          value={fleetRoot}
          onChange={(e) => patchAgents({ fleetRoot: e.target.value })}
          placeholder="auto (common parent of your projects)"
          style={{ ...inputStyle, width: 260 }}
        />
      </Row>
      <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-disabled)' }}>
        The parent directory the manager opens in. Leave blank to derive it from your configured
        projects’ common parent (else your home directory). An absolute path or a leading{' '}
        <code>~/</code>; anything else is taken literally.
      </div>

      <CheckRow
        label="Full access (workers skip approvals)"
        checked={fleetFullAccess}
        onChange={(v) => patchAgents({ fleetFullAccess: v })}
      />
      <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-disabled)' }}>
        When on, the agents the manager dispatches run with permissions bypassed — no per-action
        approval prompts, even when the dispatch does not ask for one. Faster and hands-off, but
        there is no human gate on each command. The manager still asks you before anything
        destructive or cross-repo. Off by default.{' '}
        <strong>Takes effect for newly spawned sessions.</strong> A running manager picks this up
        immediately for agents it dispatches from now on, but its OWN tool calls keep the permission
        mode it was started with — a session’s bypass is fixed when it spawns — so respawn it to
        change that. Workspacer tells you when a running manager or supervisor is affected.
      </div>
    </Section>
  );
};

export default SupervisorSection;
