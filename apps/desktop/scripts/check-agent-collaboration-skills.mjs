// Verify source assets, the generated bundle, and tsc's emitted runtime copy.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, mkdirSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const require = createRequire(import.meta.url);
const desktop = fileURLToPath(new URL('../', import.meta.url));
const scratch = mkdtempSync(join(desktop, '.agent-collaboration-skills-'));
try {
  const generated = JSON.parse(
    readFileSync(
      join(desktop, 'src/main/services/agentCollaborationSkills.generated.json'),
      'utf8',
    ),
  );
  for (const name of ['project-brief', 'spawn-agent']) {
    assert.equal(
      generated[`${name}/SKILL.md`],
      readFileSync(join(desktop, 'assets/skills', name, 'SKILL.md'), 'utf8'),
      `Regenerate agentCollaborationSkills.generated.json after changing ${name}`,
    );
  }
  const headless = readFileSync(
    resolve(desktop, '../../services/hub/cmd/brain/agent_collaboration_skills_generated.go'),
    'utf8',
  );
  for (const name of ['project-brief', 'spawn-agent'])
    assert.ok(
      headless.includes(JSON.stringify(generated[`${name}/SKILL.md`])),
      `Regenerate headless collaboration asset after changing ${name}`,
    );
  assert.ok(headless.includes('headlessAgentCollaborationSkillsVersion'));
  assert.ok(!headless.includes('headlessAgentCollaborationInstructions'));
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
  assert.deepEqual(
    require(join(dist, 'services/agentCollaborationSkills.generated.json')),
    generated,
  );
  const project = join(scratch, 'project');
  mkdirSync(project);
  const { installAgentCollaborationSkills } = require(
    join(dist, 'services/agentCollaborationSkills.js'),
  );
  const pointer = installAgentCollaborationSkills('claude', project);
  const quotedSpawnPath = pointer.match(/read ("(?:[^"\\]|\\.)+") before/)?.[1];
  assert.ok(quotedSpawnPath, `ordinary Claude spawn did not receive a skill pointer: ${pointer}`);
  const spawnPath = JSON.parse(quotedSpawnPath);
  assert.ok(spawnPath.includes(join('.workspacer', 'skills')));
  assert.equal(readFileSync(spawnPath, 'utf8'), generated['spawn-agent/SKILL.md']);
  assert.equal(installAgentCollaborationSkills('claude', project, true), '');
  assert.equal(installAgentCollaborationSkills('pi', project), '');
  console.log('Emitted ordinary-agent skills and compiled installer: passed.');
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
