// Verify the normal TypeScript distribution, not a source-tree-only installer.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, mkdirSync, readdirSync } from 'node:fs';
import { join, relative } from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
const require = createRequire(import.meta.url);
const desktop = fileURLToPath(new URL('../', import.meta.url));
const scratch = mkdtempSync(join(desktop, '.card-assets-'));
try {
  const dist = join(scratch, 'dist');
  execFileSync(
    process.execPath,
    [
      join(desktop, 'node_modules/typescript/bin/tsc'),
      '-p',
      join(desktop, 'tsconfig.main.json'),
      '--outDir',
      dist,
    ],
    { stdio: 'inherit' },
  );
  const files = require(join(dist, 'services/responseCardSkill.generated.json'));
  assert.deepEqual(
    files,
    JSON.parse(
      readFileSync(join(desktop, 'src/main/services/responseCardSkill.generated.json'), 'utf8'),
    ),
  );
  const assetRoot = join(desktop, 'assets/skills/workspacer-response-cards');
  const assets = {};
  for (const entry of readdirSync(assetRoot, { recursive: true, withFileTypes: true })) {
    if (!entry.isFile()) continue;
    const file = join(entry.parentPath, entry.name);
    assets[relative(assetRoot, file).replaceAll('\\', '/')] = readFileSync(file, 'utf8');
  }
  assert.deepEqual(
    files,
    assets,
    'Regenerate responseCardSkill.generated.json after changing assets',
  );
  const { parseHtmlCard } = require(join(dist, 'shared/htmlCard.js'));
  const examples = [
    ...files['references/examples.md'].matchAll(/```wks-html-card\n([\s\S]*?)\n```/g),
  ];
  assert.equal(examples.length, 3);
  for (const [, raw] of examples) assert.equal(parseHtmlCard(raw).ok, true);
  const project = join(scratch, 'project');
  mkdirSync(project);
  const { installResponseCardSkill } = require(join(dist, 'services/responseCardSkill.js'));
  assert.equal(installResponseCardSkill('claude', project), '');
  assert.equal(
    readFileSync(join(project, '.claude/skills/workspacer-response-cards/SKILL.md'), 'utf8'),
    files['SKILL.md'],
  );
  console.log('Emitted assets, compiled installer, and three card examples: passed.');
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
