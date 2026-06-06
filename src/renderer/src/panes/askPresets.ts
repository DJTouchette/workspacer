export interface AskPreset {
  id: string;
  label: string;
  prompt: string;
}

export const ASK_PRESETS: AskPreset[] = [
  {
    id: 'standup',
    label: 'Standup',
    prompt:
      'Give me a standup: for each agent, what it\'s working on, what it did recently, and whether it\'s blocked. Order by what needs my attention first. Reference each session as session:<id>.',
  },
  {
    id: 'triage',
    label: 'Triage',
    prompt:
      'Which agents need my attention right now and why? Rank by urgency and give the single next action for each. Reference each as session:<id>.',
  },
  {
    id: 'audit',
    label: 'Audit',
    prompt:
      'Scan all agents for risky activity (rm -rf, force pushes, writes outside the repo, leaked secrets). Report anything concerning with its session:<id>.',
  },
  {
    id: 'cost',
    label: 'Cost',
    prompt:
      'Summarize token/context usage and cost per agent right now, and call out any near their context limit. Reference each as session:<id>.',
  },
];
