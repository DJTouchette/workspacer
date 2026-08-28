/**
 * Settings → Manager & Supervisor.
 *
 * TWO agents live in this pane, and confusing them is the bug it was rebuilt to
 * stop. They are separate sessions with separate settings:
 *
 *   Fleet Manager — started from the Overview dashboard. Dispatches worker
 *                   agents into your projects and reports back. Harness:
 *                   `agents.managerProvider`.
 *   Supervisor    — started from "Ask the Fleet". Watches the agents you
 *                   already have and tells you when one needs you. Harness:
 *                   `supervisor.provider`.
 *
 * The pane used to be titled "Supervisor", with the manager's controls tacked
 * on the end under a small heading and both harness rows labelled "… agent". So
 * setting "Supervisor agent" to Codex and then launching the manager produced a
 * Claude agent, correctly, and looked like the setting had been ignored. The
 * manager now comes FIRST (it is the one people actually start), each role is a
 * self-contained block that says what starts it, and every row names its role.
 *
 * The other rule here: every control maps to something a spawn path reads, and
 * every setting a spawn path reads has a control. Both roles resolve harness
 * (main/lib/roleProviders), model and effort (main/lib/roleModels,
 * main/lib/supervisorModel) IN MAIN, so what you set here applies however the
 * agent is started — the dashboard, the palette, a phone, a hub job.
 */
import React from 'react';
import { Config } from '../../hooks/useConfig';
import type { AgentProvider } from '../../types/pane';
import HarnessModelSelect, { useModelOptions } from './HarnessModelSelect';
import HarnessEffortSelect from './HarnessEffortSelect';
import { isForeignModel } from '../../../../main/shared/modelVocabulary';
import { useProviderDetection } from '../../hooks/useProviderDetection';
import { visibleProviderOptions, NOT_INSTALLED_SUFFIX } from '../../lib/providerAvailability';
import { SUPERVISOR_PROVIDERS, MANAGER_PROVIDERS } from '../../lib/roleProviders';
import {
  Section,
  Row,
  SearchableSelect,
  SelectOption,
  ModeButton,
  CheckRow,
  inputStyle,
} from './primitives';

const hintStyle: React.CSSProperties = {
  fontSize: '0.72rem',
  color: 'var(--wks-text-disabled)',
};
const warnStyle: React.CSSProperties = { fontSize: '0.72rem', color: 'var(--wks-warning)' };

/** The heading + one-line "what starts this" that opens each role's block. */
const RoleHeading: React.FC<{ title: string; children: React.ReactNode; first?: boolean }> = ({
  title,
  children,
  first,
}) => (
  <>
    <div style={{ marginTop: first ? 4 : 26, fontWeight: 600, fontSize: '0.82rem' }}>{title}</div>
    <div style={{ ...hintStyle, margin: '4px 0 10px' }}>{children}</div>
  </>
);

interface SupervisorSectionProps {
  config: Config;
  save: (partial: Partial<Config>) => Promise<Config>;
}

const SupervisorSection: React.FC<SupervisorSectionProps> = ({ config, save }) => {
  const { detection } = useProviderDetection();

  // ── Fleet Manager ─────────────────────────────────────────────────────────
  const agents = config.agents ?? {};
  const fleetRoot = agents.fleetRoot ?? '';
  const fleetFullAccess = agents.fleetFullAccess === true;
  const managerProvider: AgentProvider = agents.managerProvider ?? 'claude';
  const visibleManagerProviders = visibleProviderOptions(MANAGER_PROVIDERS, detection, [
    managerProvider,
  ]);
  const managerProviderMissing = visibleManagerProviders.some(
    (p) => p.value === managerProvider && p.missing,
  );
  const patchAgents = (p: Partial<NonNullable<Config['agents']>>) =>
    save({ agents: { ...agents, ...p } });

  // ── Supervisor ────────────────────────────────────────────────────────────
  const sup = config.supervisor ?? {};
  const supProvider: AgentProvider = sup.provider ?? 'claude';
  const model = sup.model ?? '';
  // Per-harness, resolved the same way main does (lib/roleModels): this
  // harness's own entry first, then the legacy single field but ONLY where this
  // harness could serve it — `'sonnet'` is a claude id and must not show up as
  // codex's configured summarizer.
  const summarizerModel =
    sup.summarizerModels?.[supProvider] ??
    (isForeignModel(supProvider, sup.summarizerModel) ? '' : (sup.summarizerModel ?? ''));
  const pollSeconds = sup.pollSeconds ?? 45;

  // A harness whose CLI isn't installed can't run a supervisor, so it isn't
  // offered — except when it IS the configured one, which stays listed and
  // flagged (same treatment as a model that left the harness's catalog below:
  // a picker that silently drops its own value reads as a reset, not a
  // diagnosis).
  const visibleSupProviders = visibleProviderOptions(SUPERVISOR_PROVIDERS, detection, [
    supProvider,
  ]);
  const supProviderMissing = visibleSupProviders.some((p) => p.value === supProvider && p.missing);

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

  const patch = (p: Partial<NonNullable<Config['supervisor']>>) =>
    save({ supervisor: { ...sup, ...p } });

  /**
   * Switch harness. The model and effort fields belong to the harness they were
   * chosen on — `fable` is meaningless to codex, and codex's `xhigh` to
   * claude — so the outgoing model is filed under the outgoing provider and the
   * incoming one's remembered choice (or its default) takes its place. Flipping
   * back and forth is lossless; nothing is silently left pointing at the wrong
   * CLI. Effort needs no such shuffle: it is stored ONLY per harness.
   */
  const setProvider = (next: AgentProvider) => {
    if (next === supProvider) return;
    const models = { ...(sup.models ?? {}), [supProvider]: model };
    return patch({ provider: next, model: models[next] ?? '', models });
  };

  /** Save the model AND remember it for this harness (see setProvider). */
  const setModel = (v: string) =>
    patch({ model: v, models: { ...(sup.models ?? {}), [supProvider]: v } });

  /** Same per-harness memory as setModel, for the digest-worker model. The
   *  legacy single field is kept in step only while it is servable here, so a
   *  claude-shaped `'sonnet'` is never rewritten to mean codex. */
  const setSummarizerModel = (v: string) =>
    patch({
      summarizerModels: { ...(sup.summarizerModels ?? {}), [supProvider]: v },
      ...(!isForeignModel(supProvider, v) && { summarizerModel: v }),
    });

  return (
    <Section title="Manager & Supervisor">
      <div style={{ ...hintStyle, marginBottom: 8 }}>
        Two optional agents that work across your whole fleet rather than in one repo. Each is
        started by hand, runs on its own harness, and is configured separately below. Nothing here
        runs until you start one.
      </div>

      <RoleHeading title="Fleet Manager" first>
        Started from the <strong>Fleet Manager</strong> card on the Overview dashboard. It
        dispatches worker agents into your projects, watches them finish or block, and reports back
        to you.
      </RoleHeading>

      <Row label="Manager runs on">
        <div style={{ display: 'flex', gap: 4 }}>
          {visibleManagerProviders.map((p) => (
            <ModeButton
              key={p.value}
              label={p.missing ? `${p.label}${NOT_INSTALLED_SUFFIX}` : p.label}
              active={managerProvider === p.value}
              onClick={() => patchAgents({ managerProvider: p.value })}
            />
          ))}
        </div>
      </Row>
      {managerProviderMissing && (
        <div style={warnStyle}>
          The <strong>{managerProvider}</strong> CLI was not found on this machine — the Fleet
          Manager will fail to start on it. Install it, set its path under Settings → Session → Tool
          paths, or pick another above.
        </div>
      )}
      <div style={hintStyle}>
        The CLI hosting the manager’s own conversation. It dispatches workers on any harness either
        way. Only Claude and Codex are offered: the manager needs both an MCP client (to dispatch at
        all) and a skills folder for its own commands.{' '}
        <strong>Applies to the next manager you start.</strong> A conversation cannot move between
        CLIs, so an existing Fleet Manager card keeps the one it was started on; terminate it
        (right-click → Terminate) to start a fresh manager here.
      </div>

      <HarnessModelSelect
        provider={managerProvider}
        label="Manager model"
        value={agents.managerModels?.[managerProvider] ?? ''}
        onChange={(v) =>
          patchAgents({ managerModels: { ...(agents.managerModels ?? {}), [managerProvider]: v } })
        }
        defaultLabel={`${managerProvider} default`}
        hint={
          <>
            The model the manager’s own conversation runs on, from the models the CLI above actually
            offers. Keep this strong — it reasons about your whole fleet and writes the dispatches.
            Each CLI remembers its own choice, so switching back and forth doesn’t lose it.{' '}
            <strong>Applies to the next manager you start.</strong>
          </>
        }
      />

      <HarnessEffortSelect
        provider={managerProvider}
        label="Manager thinking effort"
        value={agents.managerEfforts?.[managerProvider] ?? ''}
        onChange={(v) =>
          patchAgents({
            managerEfforts: { ...(agents.managerEfforts ?? {}), [managerProvider]: v },
          })
        }
        hint={
          <>
            How hard the manager thinks before answering. Higher costs more and is slower. Leave it
            on the default to let the CLI decide. Each CLI has its own levels and remembers its own
            choice. <strong>Applies to the next manager you start.</strong>
          </>
        }
      />

      <Row label="Projects root">
        <input
          value={fleetRoot}
          onChange={(e) => patchAgents({ fleetRoot: e.target.value })}
          placeholder="auto (common parent of your projects)"
          style={{ ...inputStyle, width: 260 }}
        />
      </Row>
      <div style={hintStyle}>
        The parent directory the manager opens in. Leave blank to derive it from your configured
        projects’ common parent (else your home directory). An absolute path or a leading{' '}
        <code>~/</code>; anything else is taken literally.
      </div>

      <CheckRow
        label="Full access — agents the manager dispatches skip approvals"
        checked={fleetFullAccess}
        onChange={(v) => patchAgents({ fleetFullAccess: v })}
      />
      <div style={hintStyle}>
        When on, the agents the manager dispatches run with permissions bypassed — no per-action
        approval prompts, even when the dispatch does not ask for one. Faster and hands-off, but
        there is no human gate on each command. The manager still asks you before anything
        destructive or cross-repo. Off by default.{' '}
        <strong>Takes effect for newly spawned sessions.</strong> A running manager picks this up
        immediately for agents it dispatches from now on, but its OWN tool calls keep the permission
        mode it was started with — a session’s bypass is fixed when it spawns — so respawn it to
        change that. Workspacer tells you when a running manager or supervisor is affected.
      </div>

      <RoleHeading title="Supervisor">
        Started from <strong>Ask the Fleet</strong> (the launcher also lets you override the CLI for
        one launch). It watches the agents you already have, summarizes what they’re doing, and
        notifies you when one needs a decision. It does not dispatch work of its own.
      </RoleHeading>

      <Row label="Supervisor runs on">
        <div style={{ display: 'flex', gap: 4 }}>
          {visibleSupProviders.map((p) => (
            <ModeButton
              key={p.value}
              label={p.missing ? `${p.label}${NOT_INSTALLED_SUFFIX}` : p.label}
              active={supProvider === p.value}
              onClick={() => setProvider(p.value)}
            />
          ))}
        </div>
      </Row>
      {supProviderMissing && (
        <div style={warnStyle}>
          The <strong>{supProvider}</strong> CLI was not found on this machine — a supervisor
          started on it will fail to launch. Install it, set its path under Settings → Session →
          Tool paths, or pick another above.
        </div>
      )}
      <div style={hintStyle}>
        The CLI the supervisor runs on, wherever it is started from — this window, a phone, or a
        scheduled job. Codex, Copilot and OpenCode supervisors reach the fleet through their own MCP
        config; Claude remains the most battle-tested.
      </div>

      <Row label="Supervisor model">
        <SearchableSelect
          value={model}
          options={modelOptions}
          onChange={setModel}
          placeholder={supProvider === 'claude' ? 'App default' : 'Harness default'}
        />
      </Row>
      <div style={hintStyle}>
        The model the supervisor’s own conversation runs on, from the models the CLI above actually
        offers. Keep this strong — it reasons over the fleet and composes notifications. Each CLI
        remembers its own choice, so switching back and forth doesn’t lose it.
      </div>
      {modelUnknown && (
        <div style={warnStyle}>
          <strong>{model}</strong> is not in {supProvider}’s model list — a supervisor started on
          this CLI will be refused it. Pick one above, or leave it on the default.
        </div>
      )}

      <HarnessEffortSelect
        provider={supProvider}
        label="Supervisor thinking effort"
        value={sup.efforts?.[supProvider] ?? ''}
        onChange={(v) => patch({ efforts: { ...(sup.efforts ?? {}), [supProvider]: v } })}
        hint={
          <>
            How hard the supervisor thinks on each sweep. Higher costs more and is slower — it runs
            on a loop, so this multiplies. Leave it on the default to let the CLI decide. Each CLI
            has its own levels and remembers its own choice.
          </>
        }
      />

      <HarnessModelSelect
        provider={supProvider}
        label="Summarizer model"
        value={summarizerModel}
        onChange={setSummarizerModel}
        defaultLabel={`${supProvider} default`}
        hint={
          <>
            The cheap model the supervisor spawns to read transcripts and write digests. Those
            digest workers run on the <strong>same CLI as the supervisor</strong>, so this list
            follows the one above and each CLI remembers its own choice. Keep it cheap; leave it on
            the default to let that CLI pick.
          </>
        }
      />

      <Row label="Check the fleet every">
        <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
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
          <span style={hintStyle}>seconds</span>
        </div>
      </Row>
      <div style={hintStyle}>
        How often the supervisor re-sweeps the fleet for status and pending decisions. It is also
        woken immediately whenever an agent blocks, so this is the floor, not the only trigger.
      </div>

      <CheckRow
        label="Full access — the supervisor and its workers skip approvals"
        checked={sup.fullAccess === true}
        onChange={(v) => patch({ fullAccess: v })}
      />
      <div style={hintStyle}>
        When on, the supervisor and the digest workers it spawns skip approvals — no per-action
        approval prompts. Hands-off, but there is no human gate on each command. Off by default.{' '}
        <strong>Takes effect for newly spawned sessions.</strong> A running supervisor picks this up
        immediately for agents it spawns from now on, but its OWN tool calls keep the permission
        mode it was started with — a session’s bypass is fixed when it spawns — so respawn it to
        change that.
      </div>

      <div style={{ ...hintStyle, marginTop: 22, lineHeight: 1.5 }}>
        <strong>Both roles, not set here.</strong> Each runs with the full set of workspacer tools
        (the “operator” tier) — that is what lets them see and act on the fleet, and it is not
        adjustable per role. Each opens in its own place: the manager in the projects root above,
        the supervisor in <code>~/.workspacer</code>. Whether a Claude or Codex session runs as a
        visible terminal or as chat only follows that CLI’s own setting under Settings → Session,
        and the CLI path each uses follows Settings → Session → Tool paths.
      </div>
    </Section>
  );
};

export default SupervisorSection;
