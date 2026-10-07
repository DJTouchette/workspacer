// Bundle app-owned agent skills into main's normal tsc distribution, laid out
// as Claude Code plugins (one per role, see assets/skills/plugins.json).
// Installed apps must never depend on source-tree asset paths at runtime.
//
// scripts/generate-rust-launch-assets.py builds the Rust twin from the same
// sources with the same algorithm; check-agent-skill-plugins.mjs pins both.
import { createHash } from 'node:crypto';
import { readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const desktop = fileURLToPath(new URL('../', import.meta.url));
const skillsDir = resolve(desktop, 'assets/skills');

export function buildAgentSkillPlugins(root = skillsDir) {
  const plugins = JSON.parse(readFileSync(resolve(root, 'plugins.json'), 'utf8'));
  const files = {};
  const members = {};
  const instructions = {};
  for (const [plugin, def] of Object.entries(plugins)) {
    const skills = Object.keys(def.skills);
    members[plugin] = skills;
    files[`${plugin}/.claude-plugin/plugin.json`] =
      JSON.stringify({ name: plugin, description: def.description }, null, 2) + '\n';
    for (const skill of skills) {
      const dir = resolve(root, skill);
      for (const entry of readdirSync(dir, { recursive: true, withFileTypes: true })) {
        if (!entry.isFile()) continue;
        const file = resolve(entry.parentPath, entry.name);
        const rel = relative(dir, file).replaceAll('\\', '/');
        files[`${plugin}/skills/${skill}/${rel}`] = readFileSync(file, 'utf8');
      }
    }
    // `{dir}` is the plugin's skills directory and `{skill:<name>}` one skill's
    // SKILL.md; the launcher substitutes JSON-quoted absolute paths.
    const uses = Object.entries(def.skills);
    instructions[plugin] = {
      native:
        'Workspacer loads these skills into this session: ' +
        uses.map(([skill, use]) => `${skill} (${use})`).join(', ') +
        '. If one is missing from your available skills, read its SKILL.md under {dir}.',
      pointer:
        'Workspacer provides these skills: ' +
        uses
          .map(
            ([skill, use], i) =>
              `${i === uses.length - 1 ? 'and ' : ''}read {skill:${skill}} ${use}`,
          )
          .join(', ') +
        '.',
    };
  }
  const sorted = Object.fromEntries(
    Object.keys(files)
      .sort()
      .map((key) => [key, files[key]]),
  );
  const version = createHash('sha256').update(JSON.stringify(sorted)).digest('hex').slice(0, 16);
  return { version, plugins: members, instructions, files: sorted };
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  writeFileSync(
    resolve(desktop, 'src/main/services/agentSkillPlugins.generated.json'),
    `${JSON.stringify(buildAgentSkillPlugins(), null, 2)}\n`,
  );
}
