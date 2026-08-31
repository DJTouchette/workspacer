/**
 * Settings → Fleet Manager.
 *
 * ONE agent lives in this pane: the Fleet Manager, started from the Overview
 * dashboard. It dispatches worker agents into your projects, watches them
 * finish or block, and reports back. Its harness is `agents.managerProvider`.
 *
 * This pane used to carry two roles — the manager and the older fleet
 * supervisor — with two harness rows, two model rows and two full-access
 * checkboxes, which is exactly the confusion it now exists without: setting
 * "Supervisor agent" to Codex and then launching the manager produced a Claude
 * agent, correctly, and looked like the setting had been ignored. The
 * supervisor role is gone; "Ask the Fleet" now launches a plain triage-tier
 * agent and needs no settings of its own.
 *
 * The rule that stays: every control maps to something a spawn path reads, and
 * every setting a spawn path reads has a control. The manager resolves harness
 * (main/lib/roleProviders), model and effort (main/lib/roleModels) IN MAIN, so
 * what you set here applies however it is started — the dashboard, the palette,
 * a phone, a hub job.
 */
import React from 'react';
import { Config } from '../../hooks/useConfig';
import type { AgentProvider } from '../../types/pane';
import HarnessModelSelect from './HarnessModelSelect';
import HarnessEffortSelect from './HarnessEffortSelect';
import { useProviderDetection } from '../../hooks/useProviderDetection';
import { visibleProviderOptions, NOT_INSTALLED_SUFFIX } from '../../lib/providerAvailability';
import { MANAGER_PROVIDERS } from '../../lib/roleProviders';
import { Section, Row, ModeButton, CheckRow, inputStyle } from './primitives';

const hintStyle: React.CSSProperties = {
  fontSize: '0.72rem',
  color: 'var(--wks-text-disabled)',
};
const warnStyle: React.CSSProperties = { fontSize: '0.72rem', color: 'var(--wks-warning)' };

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

  return (
    <Section title="Fleet Manager">
      <div style={{ ...hintStyle, marginBottom: 14 }}>
        An optional agent that works across your whole fleet rather than in one repo, started from
        the <strong>Fleet Manager</strong> card on the Overview dashboard. It dispatches worker
        agents into your projects, watches them finish or block, and reports back to you. Nothing
        here runs until you start one.
      </div>

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
      {managerProvider === 'copilot' && (
        <div style={hintStyle}>
          Copilot is the one harness whose tool access isn’t settled until the session starts: a
          GitHub org policy can disable third-party MCP servers, and a manager without them can’t
          dispatch anything. It won’t fail quietly — the session raises an error naming the servers
          that didn’t attach.
        </div>
      )}
      <div style={hintStyle}>
        The CLI hosting the manager’s own conversation. It dispatches workers on any harness either
        way. Only Claude, Codex and GitHub Copilot are offered: the manager needs both an MCP client
        (to dispatch at all) and a skills folder for its own commands.{' '}
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
            Each CLI remembers its own choice, so switching back and forth doesn’t lose it. This is
            the only place the manager’s own model is chosen: the routing matrix picks the models
            for the workers it dispatches, and its <code>roles.supervisor</code> row is not
            consulted for this one. <strong>Applies to the next manager you start.</strong>
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
        change that. Workspacer tells you when a running manager is affected.
      </div>

      <div style={{ ...hintStyle, marginTop: 22, lineHeight: 1.5 }}>
        <strong>Not set here.</strong> The manager runs with the full set of workspacer tools (the
        “operator” tier) — that is what lets it see and act on the fleet, and it is not adjustable.
        It opens in the projects root above. Whether a Claude or Codex session runs as a visible
        terminal or as chat only follows that CLI’s own setting under Settings → Session, and the
        CLI path it uses follows Settings → Session → Tool paths.
      </div>
    </Section>
  );
};

export default SupervisorSection;
