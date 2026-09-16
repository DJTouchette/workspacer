// Bundle app-owned ordinary-agent skills into main's normal tsc distribution.
// Installed apps must never depend on source-tree asset paths at runtime.
import { readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const desktop = fileURLToPath(new URL('../', import.meta.url));
const files = {};
for (const name of ['project-brief', 'spawn-agent']) {
  files[`${name}/SKILL.md`] = readFileSync(
    resolve(desktop, 'assets/skills', name, 'SKILL.md'),
    'utf8',
  );
}
writeFileSync(
  resolve(desktop, 'src/main/services/agentCollaborationSkills.generated.json'),
  `${JSON.stringify(files, null, 2)}\n`,
);
