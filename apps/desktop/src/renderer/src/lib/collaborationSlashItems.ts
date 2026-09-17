import files from '../../../main/services/agentCollaborationSkills.generated.json';
import type { SlashItem } from './slashItems';

// Use the same assets as both installers, without writing native skill roots.
// Inserting the body also works for attached/remote sessions whose instruction
// pointer may refer to a different asset version or filesystem.
export const collaborationSlashItems: (SlashItem & { content: string })[] = Object.entries(
  files,
).map(([file, raw]) => {
  const name = file.split('/')[0];
  return {
    id: `workspacer-skill:${name}`,
    label: name,
    hint: /^description: (.+)$/m.exec(raw)?.[1],
    kind: 'skill',
    content: raw.replace(/^---\r?\n[\s\S]*?\r?\n---\r?\n/, '').trim(),
  };
});
