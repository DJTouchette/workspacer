import bundle from '../../../main/services/agentSkillPlugins.generated.json';
import type { SlashItem } from './slashItems';

// The ordinary-agent collaboration skills, from the same bundle the launchers
// hand to sessions. Inserting the body also works for attached/remote sessions
// whose own skill copy may be a different version or on another filesystem.
const COLLABORATION_SKILLS = ['project-brief', 'scheduled-jobs', 'spawn-agent'];
const files: Record<string, string> = bundle.files;

export const collaborationSlashItems: (SlashItem & { content: string })[] =
  COLLABORATION_SKILLS.map((name) => {
    const raw = files[`workspacer/skills/${name}/SKILL.md`];
    return {
      id: `workspacer-skill:${name}`,
      label: name,
      hint: /^description: (.+)$/m.exec(raw)?.[1],
      kind: 'skill',
      content: raw.replace(/^---\r?\n[\s\S]*?\r?\n---\r?\n/, '').trim(),
    };
  });
