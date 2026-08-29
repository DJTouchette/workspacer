/**
 * Which harnesses the two FLEET ROLES may run on.
 *
 * Picked in two places — Settings (the standing default) and the launcher you
 * actually start an agent from ("Ask the Fleet") — and the two lists disagreed:
 * Settings deliberately excluded Pi with a reason written out below, while the
 * Ask launcher offered it, so the one picker a user touches at launch time could
 * start a harness the settings pane says is impossible. One list, imported by
 * both.
 *
 * These are role-eligibility lists, not availability: what a machine actually
 * has installed is layered on top by lib/providerAvailability.
 */

/** A harness offered for a fleet role, with the label the UI shows. */
export interface RoleProviderOption {
  value: 'claude' | 'codex' | 'copilot' | 'opencode';
  label: string;
}

/**
 * Harnesses the SUPERVISOR role is verified on. Pi is deliberately absent: the
 * supervisor's whole job is watching the fleet through the workspacer MCP
 * facade and notifying you, but pi core ships no MCP client at all — `pi.rs`
 * warns facade tools are unavailable to it, `managedSpawn.ts` refuses to mint
 * it a facade token (`provider !== 'pi'`), and `agentSkillsRoot` returns null
 * for it so it never gets the /supervise skill either. A "Pi supervisor" would
 * run on role instructions alone with no way to observe or coordinate
 * anything — the same failure mode MANAGER_PROVIDERS below already excludes Pi
 * (and OpenCode) to avoid.
 */
export const SUPERVISOR_PROVIDERS: readonly RoleProviderOption[] = [
  { value: 'claude', label: 'Claude' },
  { value: 'codex', label: 'Codex' },
  // Copilot CLI has a first-class MCP client (servers ride in on
  // `--additional-mcp-config`, no config file to write), so the facade attaches
  // and a supervisor can observe and coordinate. The one caveat is that its
  // capability surface is DYNAMIC: a GitHub org policy can disable third-party
  // MCP servers, and the adapter raises a session error when that happens
  // rather than letting a toolless supervisor pass for a working one.
  { value: 'copilot', label: 'GitHub Copilot' },
  { value: 'opencode', label: 'OpenCode' },
];

/**
 * Harnesses the FLEET MANAGER role is verified on. Still narrower than
 * SUPERVISOR_PROVIDERS: the manager needs an MCP client to dispatch at all, and
 * a personal-skills directory for /standup, /checkpoint and /handoff. Listing a
 * harness that silently loses half the role is the failure mode this picker
 * exists to avoid, so a value earns its place here by having BOTH.
 *
 *  - claude  → `~/.claude/skills`, facade via `--mcp-config`.
 *  - codex   → `$CODEX_HOME/skills`, facade via `-c mcp_servers.workspacer.url`.
 *  - copilot → `~/.copilot/skills`, facade via `--additional-mcp-config`
 *    (`lib/agentSkills`, `providers/copilot.rs`). Same SKILL.md format on all
 *    three. Copilot is the one harness whose MCP surface is not decidable
 *    ahead of time — a GitHub org policy can disable third-party MCP servers —
 *    but that failure ANNOUNCES itself: the adapter checks
 *    `session.mcp_servers_loaded` and raises a session error rather than
 *    letting a toolless manager pass for a working one, so the bad case is
 *    visible instead of silent. Probed live against CLI 1.0.81: an HTTP MCP
 *    server registered this way connects, lists and executes its tools.
 *
 * OpenCode is absent for the reason SUPERVISOR_PROVIDERS' comment gives for Pi,
 * one step milder: it has an MCP client but no verified personal-skills path
 * (`agentSkillsRoot` returns null), so a manager there would come up without
 * /standup, /checkpoint or /handoff.
 */
export const MANAGER_PROVIDERS: readonly RoleProviderOption[] = [
  { value: 'claude', label: 'Claude' },
  { value: 'codex', label: 'Codex' },
  { value: 'copilot', label: 'GitHub Copilot' },
];
