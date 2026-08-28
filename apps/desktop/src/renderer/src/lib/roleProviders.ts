/**
 * Which harnesses the two FLEET ROLES may run on.
 *
 * Both roles are picked in two places — Settings (the standing default) and the
 * launcher you actually start them from ("Ask the Fleet" for the supervisor) —
 * and the two lists disagreed: Settings deliberately excluded Pi with a reason
 * written out below, while the Ask launcher offered it, so the one picker a user
 * touches at launch time could start a supervisor the settings pane says is
 * impossible. One list, imported by both.
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
 * Harnesses the FLEET MANAGER role is verified on. Narrower than
 * SUPERVISOR_PROVIDERS on purpose: the manager needs an MCP client to dispatch
 * at all, and a personal-skills directory for /standup, /checkpoint and
 * /handoff. Claude and Codex have both (`~/.claude/skills` /
 * `$CODEX_HOME/skills`, identical SKILL.md format); listing a harness that
 * silently loses half the role is the failure mode this picker exists to avoid.
 */
export const MANAGER_PROVIDERS: readonly RoleProviderOption[] = [
  { value: 'claude', label: 'Claude' },
  { value: 'codex', label: 'Codex' },
];
