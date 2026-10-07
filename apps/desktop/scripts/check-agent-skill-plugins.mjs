// Verify the agent skill plugins end to end: the generated bundle against its
// source assets, the Rust twin against the bundle, and tsc's emitted runtime
// (bundle copy + launcher) against a scratch home and project.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { buildAgentSkillPlugins } from './gen-agent-skill-plugins.mjs';

const require = createRequire(import.meta.url);
const desktop = fileURLToPath(new URL('../', import.meta.url));
const generated = JSON.parse(
  readFileSync(join(desktop, 'src/main/services/agentSkillPlugins.generated.json'), 'utf8'),
);
assert.deepEqual(
  generated,
  buildAgentSkillPlugins(),
  'Regenerate agentSkillPlugins.generated.json after changing assets/skills',
);
const rust = JSON.parse(
  readFileSync(resolve(desktop, '../../services/hub-rs/assets/launch-instructions.json'), 'utf8'),
);
for (const key of ['version', 'plugins', 'instructions', 'files']) {
  assert.deepEqual(
    rust[key],
    generated[key],
    `Rust launch assets disagree on ${key}; run scripts/generate-rust-launch-assets.py`,
  );
}

const scratch = mkdtempSync(join(desktop, '.agent-skill-plugins-'));
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
  assert.deepEqual(require(join(dist, 'services/agentSkillPlugins.generated.json')), generated);

  const { parseHtmlCard } = require(join(dist, 'shared/htmlCard.js'));
  const examples = [
    ...generated.files[
      'workspacer/skills/workspacer-response-cards/references/examples.md'
    ].matchAll(/```wks-html-card\n([\s\S]*?)\n```/g),
  ];
  assert.equal(examples.length, 3);
  for (const [, raw] of examples) assert.equal(parseHtmlCard(raw).ok, true);

  const home = join(scratch, 'home');
  const project = join(scratch, 'project');
  mkdirSync(home);
  mkdirSync(project);
  const { prepareAgentSkills } = require(join(dist, 'services/agentSkillPlugins.js'));
  const root = join(home, '.workspacer', 'agent-skills', generated.version);

  const claude = prepareAgentSkills('claude', project, { home });
  assert.deepEqual(claude.args, ['--plugin-dir', join(root, 'workspacer')]);
  for (const [rel, body] of Object.entries(generated.files)) {
    assert.equal(readFileSync(join(root, ...rel.split('/')), 'utf8'), body, rel);
  }
  assert.ok(claude.instruction.includes(JSON.stringify(join(root, 'workspacer', 'skills'))));

  const codex = prepareAgentSkills('codex', project, { home, manager: true });
  assert.deepEqual(codex.args, []);
  assert.deepEqual(codex.skillRoots, [join(root, 'workspacer-fleet', 'skills')]);

  const copilot = prepareAgentSkills('copilot', project, { home });
  assert.ok(
    copilot.instruction.includes(
      JSON.stringify(join(root, 'workspacer', 'skills', 'spawn-agent', 'SKILL.md')),
    ),
  );
  assert.deepEqual(prepareAgentSkills('pi', project, { home }), {
    args: [],
    skillRoots: [],
    instruction: '',
  });
  // Nothing lands in the project or a harness discovery root.
  for (const dir of ['.workspacer', '.claude', '.agents']) {
    assert.equal(existsSync(join(project, dir)), false, dir);
  }
  console.log('Agent skill plugin bundle, Rust twin and compiled launcher: passed.');
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
