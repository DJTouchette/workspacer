// Bundle repo-owned skill files into main's normal tsc distribution. Never read
// source-tree paths at runtime: installed apps contain only dist + dependencies.
import { readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { resolve, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
const desktop = fileURLToPath(new URL('../', import.meta.url));
const root = resolve(desktop, 'assets/skills/workspacer-response-cards');
const files = {};
for (const entry of readdirSync(root, { recursive: true, withFileTypes: true })) {
  if (!entry.isFile()) continue;
  const file = resolve(entry.parentPath, entry.name);
  files[relative(root, file).replaceAll('\\', '/')] = readFileSync(file, 'utf8');
}
writeFileSync(
  resolve(desktop, 'src/main/services/responseCardSkill.generated.json'),
  JSON.stringify(files, null, 2) + '\n',
);
