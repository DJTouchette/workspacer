/**
 * The Workspacer guide — an in-app tour guide backed by a REAL Claude agent
 * with the workspacer MCP facade at the 'triage' tier. That tier includes the
 * UI-navigation tools (focus_agent / open_pane / open_browser / open_plugin /
 * open_spawn_dialog), so the guide can literally walk the user around the app
 * while explaining it. The facade's own `help` tool (topic "ui") documents the
 * tour workflow, so this preamble stays short and cannot drift from the tools.
 *
 * Shared between the welcome card's tour chips, the Guide pane, and
 * useAgentManager's spawnGuide — one source for the name, the presets, and the
 * kickoff framing.
 */

/** Display name of the guide's agent workspace. Also how an already-running
 *  guide is recognized (spawnGuide passes it as the user-set name, which
 *  protects it from auto-titling). */
export const GUIDE_AGENT_NAME = 'Workspacer Guide';

/** Role framing prepended to the guide's first message. Deliberately compact:
 *  detailed tool guidance lives behind the facade's `help` tool. */
const GUIDE_PREAMBLE =
  'You are the Workspacer guide — a friendly in-app tour guide for someone new to ' +
  'Workspacer, the desktop cockpit for running many coding agents side by side. ' +
  'You have workspacer MCP tools (mcp__workspacer__*): call the "help" tool first, ' +
  'including its "ui" topic. When asked for a tour, narrate each step in chat and then ' +
  'use the UI tools (open_pane, focus_agent, open_spawn_dialog, …) so the user sees ' +
  'what you describe. Keep answers short, concrete, and beginner-friendly, and end ' +
  'each answer by suggesting one thing to try or ask next. The user asks:';

/** Compose the guide's first (auto-sent) message from a user question. */
export function buildGuideKickoff(question: string): string {
  return `${GUIDE_PREAMBLE}\n\n${question.trim()}`;
}

export interface GuidePreset {
  id: string;
  label: string;
  prompt: string;
}

/** Preloaded questions — surfaced as chips on the welcome card and in the
 *  Guide pane. Prompts are phrased so the guide both explains AND shows. */
export const GUIDE_PRESETS: GuidePreset[] = [
  {
    id: 'tour',
    label: 'Show me around',
    prompt:
      'Give me a guided tour of Workspacer: walk me through the main surfaces one at a time, opening each one as you describe it.',
  },
  {
    id: 'agents',
    label: 'What can agents do?',
    prompt:
      'Explain how agents work in Workspacer — spawning them, the different providers, permission modes, and how I watch or approve what they do.',
  },
  {
    id: 'jobs',
    label: 'How do jobs work?',
    prompt:
      'Explain the Jobs system — recurring and one-off jobs, what they can run — and show me where to set one up.',
  },
  {
    id: 'remote',
    label: 'Use it from my phone?',
    prompt:
      'Explain remote access — pairing my phone, the mobile app, and sharing over the hub — and show me where to set it up.',
  },
];
