/**
 * Functional check for the library's .claude integration:
 * read, frontmatter-preserving edit, create, delete — against a temp fixture.
 *
 *   npm run build:main && node scripts/test-library-claude.js
 */
const fs = require('fs');
const os = require('os');
const path = require('path');

const { libraryService } = require('../dist/main/services/libraryService');

let failures = 0;
const check = (name, cond) => {
  console.log(`${cond ? 'PASS' : 'FAIL'}  ${name}`);
  if (!cond) failures++;
};

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-lib-test-'));

// Fixture: one skill (with extra frontmatter + a resource file), one agent (tools/model keys)
const skillDir = path.join(root, '.claude', 'skills', 'deploy-checklist');
fs.mkdirSync(skillDir, { recursive: true });
fs.writeFileSync(path.join(skillDir, 'SKILL.md'), [
  '---',
  'name: deploy-checklist',
  'description: Steps before deploying',
  'metadata:',
  '  version: 2',
  '---',
  '',
  'Always run the smoke tests first.',
].join('\n'));
fs.writeFileSync(path.join(skillDir, 'resource.txt'), 'extra file');

const agentsDir = path.join(root, '.claude', 'agents');
fs.mkdirSync(agentsDir, { recursive: true });
fs.writeFileSync(path.join(agentsDir, 'reviewer.md'), [
  '---',
  'name: reviewer',
  'description: Reviews PRs',
  'tools: Read, Grep, Glob',
  'model: sonnet',
  '---',
  '',
  'You are a strict code reviewer.',
].join('\n'));

// 1. list() surfaces both, tagged scope=claude
const items = libraryService.list(root).filter((i) => i.scope === 'claude');
const skill = items.find((i) => i.kind === 'skill');
const agent = items.find((i) => i.kind === 'agent');
check('skill listed with scope=claude', !!skill && skill.id === 'deploy-checklist' && skill.title === 'deploy-checklist');
check('skill description parsed', skill?.description === 'Steps before deploying');
check('skill body parsed', skill?.body.startsWith('Always run the smoke tests'));
check('agent listed with scope=claude', !!agent && agent.id === 'reviewer' && agent.kind === 'agent');

// 2. edit the agent — tools/model must survive the round trip
libraryService.save({
  scope: 'claude', id: 'reviewer', title: 'reviewer', kind: 'agent',
  description: 'Reviews PRs thoroughly', body: 'You are a strict but kind code reviewer.', cwd: root,
});
const agentRaw = fs.readFileSync(path.join(agentsDir, 'reviewer.md'), 'utf-8');
check('agent description updated', agentRaw.includes('Reviews PRs thoroughly'));
check('agent tools preserved', agentRaw.includes('tools: Read, Grep, Glob'));
check('agent model preserved', agentRaw.includes('model: sonnet'));
check('agent body updated', agentRaw.includes('strict but kind'));

// 3. edit the skill — extra metadata key must survive
libraryService.save({
  scope: 'claude', id: 'deploy-checklist', title: 'deploy-checklist', kind: 'skill',
  description: 'Steps before deploying (updated)', body: 'Run smoke tests, then check the dashboard.', cwd: root,
});
const skillRaw = fs.readFileSync(path.join(skillDir, 'SKILL.md'), 'utf-8');
check('skill metadata preserved', /metadata:\r?\n\s+version: 2/.test(skillRaw));
check('skill resource file untouched', fs.existsSync(path.join(skillDir, 'resource.txt')));

// 4. create a brand-new skill from the pane
libraryService.save({
  scope: 'claude', title: 'New Release Notes', kind: 'skill',
  description: 'Draft release notes', body: 'Summarize merged PRs since the last tag.', cwd: root,
});
const newSkill = path.join(root, '.claude', 'skills', 'new-release-notes', 'SKILL.md');
check('new skill created at .claude/skills/<id>/SKILL.md', fs.existsSync(newSkill));
check('new skill uses name frontmatter', fs.readFileSync(newSkill, 'utf-8').startsWith('---\nname: New Release Notes'));

// 5. remove: agent file unlinked, skill folder removed recursively
libraryService.remove('claude', 'reviewer', root, 'agent');
check('agent removed', !fs.existsSync(path.join(agentsDir, 'reviewer.md')));
libraryService.remove('claude', 'deploy-checklist', root, 'skill');
check('skill folder removed', !fs.existsSync(skillDir));

// 6. a bare project must not get .claude dirs created as a watch side effect
const bare = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-bare-test-'));
libraryService.list(bare);
check('no .claude dir created in bare project', !fs.existsSync(path.join(bare, '.claude')));
check('.workspacer library dir is still auto-created (existing behavior)', fs.existsSync(path.join(bare, '.workspacer', 'library')));

fs.rmSync(root, { recursive: true, force: true });
fs.rmSync(bare, { recursive: true, force: true });
console.log(failures === 0 ? '\nall checks passed' : `\n${failures} check(s) FAILED`);
process.exit(failures === 0 ? 0 : 1);
