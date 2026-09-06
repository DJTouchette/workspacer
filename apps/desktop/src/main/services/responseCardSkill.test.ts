import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import {
  installResponseCardSkill,
  RESPONSE_CARD_SKILL_NAME,
  responseCardSkillRoot,
} from './responseCardSkill';
let project: string;
beforeEach(() => {
  project = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-skill-project-'));
});
afterEach(() => fs.rmSync(project, { recursive: true, force: true }));
const skill = (root: string) =>
  path.join(project, root, 'skills', RESPONSE_CARD_SKILL_NAME, 'SKILL.md');
describe('project-only product skill installation', () => {
  it.each([
    ['claude', '.claude'],
    ['codex', '.agents'],
  ] as const)('installs %s package into %s', (provider, root) => {
    expect(installResponseCardSkill(provider, project)).toBe('');
    expect(fs.readFileSync(skill(root), 'utf8')).toContain('name: workspacer-response-cards');
    expect(fs.readdirSync(path.join(path.dirname(skill(root)), 'references')).sort()).toEqual([
      'examples.md',
      'interactivity.md',
      'schema.md',
    ]);
    const mtime = fs.statSync(skill(root)).mtimeMs;
    installResponseCardSkill(provider, project);
    expect(fs.statSync(skill(root)).mtimeMs).toBe(mtime);
    expect(fs.existsSync(path.join(project, '.codex'))).toBe(false);
  });
  it('never overwrites user edits, even with the old generated marker', () => {
    installResponseCardSkill('claude', project);
    const mine = '<!-- workspacer:generated response-cards skill -->\nmy edits';
    fs.writeFileSync(skill('.claude'), mine);
    const note = installResponseCardSkill('claude', project);
    expect(fs.readFileSync(skill('.claude'), 'utf8')).toBe(mine);
    expect(note).toContain(path.join(responseCardSkillRoot(project), 'SKILL.md'));
  });
  it('refuses symlink destinations without writing through them', () => {
    fs.mkdirSync(path.join(project, 'elsewhere'));
    fs.symlinkSync(path.join(project, 'elsewhere'), path.join(project, '.agents'));
    installResponseCardSkill('codex', project);
    expect(fs.readdirSync(path.join(project, 'elsewhere'))).toEqual([]);
  });
  it('refuses the personal home and filesystem root', () => {
    expect(installResponseCardSkill('claude', os.homedir())).toBe('');
    expect(installResponseCardSkill('codex', path.parse(project).root)).toBe('');
  });
  it('provides an existing product file pointer for other adapters', () => {
    const note = installResponseCardSkill('opencode', project);
    expect(note).toContain(path.join(responseCardSkillRoot(project), 'SKILL.md'));
    expect(fs.existsSync(path.join(responseCardSkillRoot(project), 'SKILL.md'))).toBe(true);
    expect(fs.existsSync(path.join(project, '.opencode'))).toBe(false);
  });
});
