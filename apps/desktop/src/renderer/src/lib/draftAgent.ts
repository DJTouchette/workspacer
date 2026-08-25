/**
 * "Draft this with an agent" — the launcher behind the Settings buttons that
 * open a real agent session already pointed at the thing you were looking at.
 *
 * Four properties are load-bearing, and every one of them is pinned HERE, in
 * code, rather than left to whichever surface pressed the button:
 *
 * 1. THE PROMPT IS OWNED BY THE APP. The bus event carries a brief ID, never
 *    text. There is no parameter a caller could pass a string through, so
 *    nothing that came off disk — a committable `<cwd>/.workspacer/library`
 *    item, a repo file, a plugin's manifest — can become the thing an agent is
 *    launched with. See `components/LibraryHost.tsx` for the other half of
 *    that rule: the path that DOES carry off-disk text pre-fills the composer
 *    and never sends it.
 *
 * 2. THE TEXT IS PRE-FILLED, NEVER AUTO-SENT. `buildDraftSpawn` returns
 *    `initialPrompt`, and its return type has no `kickoffMessage` field at
 *    all, so auto-sending from this path is a type error rather than a
 *    judgement call. These prompts are app-owned, so auto-send would be
 *    defensible on the Guide's reasoning — we do not take it, because "this
 *    path never auto-sends" is one rule that holds no matter what a later
 *    brief contains, and because the user reading the prompt before it runs is
 *    the point of the affordance.
 *
 * 3. THE TIER IS PINNED PER BRIEF. `toolScope` is the one genuinely ungated
 *    spawn field — nothing downstream checks a grant for it — so it is a
 *    constant on an app-owned brief and never something a stored item names
 *    about itself.
 *
 * 4. THE CWD IS PINNED AT THE CALL SITE. `buildDraftSpawn` takes the home
 *    directory as an argument and has no access to any agent's cwd. These
 *    agents work on your SETTINGS, not on whatever repo happened to be in
 *    front of you when you opened Settings.
 *
 * What is deliberately NOT here: any notion of a "proposal". Jobs has one
 * because a job is argv that runs later with nobody watching; themes and
 * keybindings do not, because `save_config` is a direct write and inventing a
 * review step for a colour scheme would be ceremony. A brief that wants a
 * review step says so in its own prompt, by naming the domain's own propose
 * capability (`propose_job`). There is no shared proposal machinery, and
 * there should not be one until a second domain actually needs it.
 */

/** Facade tiers a brief may ask for. Spelled out rather than imported so this
 *  module has no dependency on the spawn plumbing. */
export type DraftToolScope = 'view' | 'triage' | 'operator';

export type DraftBriefId = 'jobs' | 'appearance' | 'keybindings';

export interface DraftBrief {
  id: DraftBriefId;
  /** Fixed workspace name. A running one is reused rather than duplicated,
   *  the way the Guide and the Fleet Manager are. */
  agentName: string;
  /** Button label. */
  label: string;
  /** The line under the button. Says what the agent will do and, for the
   *  briefs that write settings directly, how to undo it. */
  hint: string;
  /** MCP facade tier. Pinned here; see property 3 above. */
  toolScope: DraftToolScope;
  /** The kickoff text, pre-filled into the composer for the user to read. */
  prompt: string;
}

/**
 * Why every brief below is `operator`: the capabilities they need
 * (`jobs.propose`, `config.save`) match no scoped tier's allowlist, so
 * proposing a job or changing a setting requires an operator token today. That
 * is a real cost — operator also carries spawn, send_message and write_file —
 * and it is what keeps this list short. It is acceptable here only because
 * every prompt is app-owned; a brief whose text came from anywhere else would
 * not be.
 */
export const DRAFT_BRIEFS: Record<DraftBriefId, DraftBrief> = {
  jobs: {
    id: 'jobs',
    agentName: 'Job drafter',
    label: 'Draft one with an agent',
    hint: 'Opens an agent that works out the schedule with you and proposes the job. It lands switched off for you to approve here.',
    toolScope: 'operator',
    prompt: [
      'You are drafting a scheduled JOB for the user of Workspacer, the desktop cockpit they run their coding agents in.',
      '',
      'You have Workspacer MCP tools (mcp__workspacer__*). Call the "help" tool with topic "jobs" FIRST: it documents the job spec — the trigger shapes (interval, daily, once, manual) and the action shapes (spawn an agent with a prompt, run a shell command, call a bus capability) — and you should write the spec from that rather than from memory.',
      '',
      'Then:',
      '1. Ask what they want to happen and when. A couple of short questions, not an interrogation. If a shell command is involved, get the exact command from them rather than guessing at one.',
      '2. Call propose_job with the spec.',
      '3. Tell them plainly what happened: it landed as a PROPOSAL. It is switched off, it will not run, and it is waiting in Settings then Jobs, where they read the trigger and the action and press Approve. Do not say it is scheduled, because it is not.',
      '',
      'You cannot arm a job yourself and you should not try. A job is argv that runs later with nobody watching, which is exactly why arming it is the human’s to do and not yours.',
    ].join('\n'),
  },
  appearance: {
    id: 'appearance',
    agentName: 'Appearance helper',
    label: 'Change it with an agent',
    hint: 'Opens an agent that edits your theme settings directly. Undo is here: pick another theme, or delete the custom one it made.',
    toolScope: 'operator',
    prompt: [
      'You are changing the APPEARANCE settings of Workspacer, the desktop cockpit the user runs their coding agents in.',
      '',
      'You have Workspacer MCP tools (mcp__workspacer__*). get_config returns the whole config; save_config takes a PARTIAL patch that is deep-merged into it, so send only the keys you are changing.',
      '',
      'What lives where:',
      '- ui.theme is the active theme id, either a built-in name or "custom:<slug>".',
      '- ui.customThemes is a map of theme id to spec, and it is the ONE exception to the deep merge: it is replaced WHOLESALE. If you write it, read the current map first with get_config and send it back complete plus your addition, or every other custom theme the user has is gone.',
      '- The rest of the ui block (font scale, corners, density and so on) merges normally.',
      '',
      'Ask what they are after, make the change, then say exactly which keys you set. The change applies live, so tell them to look at the window. Say how to undo it too: another theme from the picker in Settings then Appearance, or deleting the custom theme you added.',
      '',
      'Do not restyle things they did not ask about, and do not rewrite a custom theme they already have without saying so first.',
    ].join('\n'),
  },
  keybindings: {
    id: 'keybindings',
    agentName: 'Keybinding helper',
    label: 'Change them with an agent',
    hint: 'Opens an agent that edits your bindings directly. Undo is here: reset one action, or reset everything to the preset.',
    toolScope: 'operator',
    prompt: [
      'You are changing the KEYBINDINGS of Workspacer, the desktop cockpit the user runs their coding agents in.',
      '',
      'You have Workspacer MCP tools (mcp__workspacer__*). get_config returns the whole config; save_config takes a PARTIAL patch that is deep-merged into it, so send only the bindings you are changing.',
      '',
      'What lives where:',
      '- keybindings.shortcuts is a map of action name to ONE combo string. An action carries exactly one binding; there is no alternates list, so a second combo replaces the first.',
      '- keybindings.presetId names the preset the user started from (VS Code, Vim, JetBrains). Leave it alone unless they ask to switch wholesale — switching a preset overwrites bindings.',
      '- "mod" in a combo means Cmd on macOS and Ctrl elsewhere. Prefer it over naming either one.',
      '',
      'Read the CURRENT shortcuts before you propose anything, so you are rebinding actions that actually exist rather than inventing names, and check you are not handing two actions the same combo.',
      '',
      'Ask what they want bound, make the change, then list the actions you touched and their new combos. Say how to undo it: Settings then Keybindings resets a single action, or resets everything back to the preset.',
    ].join('\n'),
  },
};

/** The exact spawn options a draft launch uses. Note what is absent: there is
 *  no `kickoffMessage`, so the auto-send path is closed by the type. */
export interface DraftSpawnOptions {
  cwd: string;
  name: string;
  provider: 'claude';
  transport: 'stream';
  toolScope: DraftToolScope;
  initialPrompt: string;
}

/**
 * Build the spawn for a brief.
 *
 * `home` is the app's own directory (getSupervisorHome, ~/.workspacer) and is
 * the ONLY source of cwd here: this function cannot see the focused agent, so
 * a draft session can never inherit some repo's directory by accident.
 */
export function buildDraftSpawn(brief: DraftBrief, home: string): DraftSpawnOptions {
  return {
    cwd: home,
    name: brief.agentName,
    // Chat-first, like the Guide and the Fleet Manager: this is a conversation,
    // not a terminal session.
    provider: 'claude',
    transport: 'stream',
    toolScope: brief.toolScope,
    // PRE-FILL. The user reads it, edits it if they want, and presses Enter.
    initialPrompt: brief.prompt,
  };
}

// ── Bus ──────────────────────────────────────────────────────────────────────

export const DRAFT_AGENT_EVENT = 'wks:draft-with-agent';

export interface DraftAgentDetail {
  id: DraftBriefId;
}

/** Ask the DraftWithAgentHost to open the agent for a brief. The ID is the
 *  whole payload — see property 1 in the module comment. */
export function requestDraftAgent(id: DraftBriefId): void {
  window.dispatchEvent(new CustomEvent(DRAFT_AGENT_EVENT, { detail: { id } }));
}
