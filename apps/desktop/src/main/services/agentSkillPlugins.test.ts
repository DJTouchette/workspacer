import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  AGENT_SKILLS_VERSION,
  agentSkillBundleRoot,
  materializeAgentSkills,
  prepareAgentSkills,
} from './agentSkillPlugins';
import bundle from './agentSkillPlugins.generated.json';

const files: Record<string, string> = bundle.files;
const dirs: string[] = [];
function scratch(): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-agent-skills-'));
  dirs.push(dir);
  return dir;
}
function write(file: string, body: string): void {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, body);
}

afterEach(() => {
  for (const dir of dirs.splice(0)) fs.rmSync(dir, { recursive: true, force: true });
  vi.restoreAllMocks();
});

describe('materializeAgentSkills', () => {
  it('writes every bundled file under the versioned root, matching the Rust twin', () => {
    const home = scratch();
    const root = materializeAgentSkills(home)!;
    expect(root).toBe(path.join(home, '.workspacer', 'agent-skills', AGENT_SKILLS_VERSION));
    for (const [rel, body] of Object.entries(files)) {
      expect(fs.readFileSync(path.join(root, ...rel.split('/')), 'utf8')).toBe(body);
    }
    const rust = JSON.parse(
      fs.readFileSync(
        path.resolve(__dirname, '../../../../../services/hub-rs/assets/launch-instructions.json'),
        'utf8',
      ),
    );
    expect(rust.version).toBe(AGENT_SKILLS_VERSION);
  });

  it('restores an altered or missing file and leaves no temporaries', () => {
    const home = scratch();
    const root = materializeAgentSkills(home)!;
    const spawn = path.join(root, 'workspacer', 'skills', 'spawn-agent', 'SKILL.md');
    fs.writeFileSync(spawn, 'tampered');
    fs.rmSync(path.join(root, 'workspacer-fleet', 'skills', 'handoff', 'SKILL.md'));
    expect(materializeAgentSkills(home)).toBe(root);
    expect(fs.readFileSync(spawn, 'utf8')).toBe(files['workspacer/skills/spawn-agent/SKILL.md']);
    expect(fs.readdirSync(path.dirname(spawn))).toEqual(['SKILL.md']);
  });

  it('refuses a symlinked bundle directory without writing through it', () => {
    vi.spyOn(console, 'warn').mockImplementation(() => {});
    const home = scratch();
    const outside = scratch();
    fs.symlinkSync(outside, path.join(home, '.workspacer'));
    expect(materializeAgentSkills(home)).toBeNull();
    expect(fs.readdirSync(outside)).toEqual([]);
  });
});

describe('prepareAgentSkills', () => {
  const quoted = (...parts: string[]) => JSON.stringify(path.join(...parts));

  it('hands Claude the role plugin by --plugin-dir', () => {
    const home = scratch();
    const cwd = scratch();
    const launch = prepareAgentSkills('claude', cwd, { home });
    const plugin = path.join(agentSkillBundleRoot(home), 'workspacer');
    expect(launch.args).toEqual(['--plugin-dir', plugin]);
    expect(launch.skillRoots).toEqual([]);
    expect(launch.instruction).toContain('spawn-agent (before spawning child agents)');
    expect(launch.instruction).toContain(quoted(plugin, 'skills'));
    expect(launch.instruction).not.toMatch(/\{(dir|skill:)/);
  });

  it('hands Codex the manager plugin skills as app-server roots', () => {
    const home = scratch();
    const launch = prepareAgentSkills('codex', scratch(), { home, manager: true });
    const skills = path.join(agentSkillBundleRoot(home), 'workspacer-fleet', 'skills');
    expect(launch.args).toEqual([]);
    expect(launch.skillRoots).toEqual([skills]);
    expect(launch.instruction).toContain('standup (for an on-demand fleet status digest)');
    expect(launch.instruction).not.toContain('spawn-agent');
  });

  it.each(['copilot', 'opencode'] as const)('names each SKILL.md for %s', (provider) => {
    const home = scratch();
    const launch = prepareAgentSkills(provider, scratch(), { home });
    const skills = path.join(agentSkillBundleRoot(home), 'workspacer', 'skills');
    expect(launch.args).toEqual([]);
    expect(launch.skillRoots).toEqual([]);
    for (const name of [
      'spawn-agent',
      'project-brief',
      'scheduled-jobs',
      'workspacer-response-cards',
    ]) {
      expect(launch.instruction).toContain(quoted(skills, name, 'SKILL.md'));
    }
  });

  it('uses pointers when the caller says the harness cannot load skills itself', () => {
    const home = scratch();
    const launch = prepareAgentSkills('codex', scratch(), { home, loading: 'pointer' });
    expect(launch.skillRoots).toEqual([]);
    expect(launch.instruction).toMatch(/^Workspacer provides these skills: read "/);
  });

  it('gives Pi nothing and writes nothing', () => {
    const home = scratch();
    expect(prepareAgentSkills('pi', scratch(), { home })).toEqual({
      args: [],
      skillRoots: [],
      instruction: '',
    });
    expect(fs.existsSync(path.join(home, '.workspacer'))).toBe(false);
  });

  it('degrades to no skills, or throws when strict, if the bundle cannot be written', () => {
    vi.spyOn(console, 'warn').mockImplementation(() => {});
    const home = scratch();
    fs.writeFileSync(path.join(home, '.workspacer'), 'not a directory');
    expect(prepareAgentSkills('claude', scratch(), { home }).args).toEqual([]);
    expect(() => prepareAgentSkills('codex', scratch(), { home, strict: true })).toThrow(
      'could not be prepared',
    );
  });

  it('never writes into the project or a harness discovery root', () => {
    const home = scratch();
    const cwd = scratch();
    for (const provider of ['claude', 'codex', 'copilot'] as const) {
      prepareAgentSkills(provider, cwd, { home });
      prepareAgentSkills(provider, cwd, { home, manager: true });
    }
    expect(fs.readdirSync(cwd)).toEqual([]);
    expect(fs.existsSync(path.join(home, '.claude'))).toBe(false);
    expect(fs.existsSync(path.join(home, '.codex'))).toBe(false);
  });
});

describe('legacy cleanup', () => {
  it('removes only exact project copies older builds wrote', () => {
    const home = scratch();
    const cwd = scratch();
    const spawnBody = files['workspacer/skills/spawn-agent/SKILL.md'];
    write(path.join(cwd, '.claude/skills/spawn-agent/SKILL.md'), spawnBody);
    write(
      path.join(cwd, '.agents/skills/workspacer-response-cards/references/schema.md'),
      files['workspacer/skills/workspacer-response-cards/references/schema.md'],
    );
    write(path.join(cwd, '.claude/skills/project-brief/SKILL.md'), 'user owned');
    write(path.join(cwd, '.workspacer/skills/8480e9fb4e9c36ed/spawn-agent/SKILL.md'), spawnBody);
    write(path.join(cwd, '.workspacer/skills/feedface/spawn-agent/SKILL.md'), 'edited');
    write(path.join(cwd, '.workspacer/brief.md'), 'keep');
    prepareAgentSkills('claude', cwd, { home });
    expect(fs.existsSync(path.join(cwd, '.claude/skills/spawn-agent'))).toBe(false);
    expect(fs.existsSync(path.join(cwd, '.agents/skills'))).toBe(false);
    expect(fs.readFileSync(path.join(cwd, '.claude/skills/project-brief/SKILL.md'), 'utf8')).toBe(
      'user owned',
    );
    expect(fs.existsSync(path.join(cwd, '.workspacer/skills/8480e9fb4e9c36ed'))).toBe(false);
    expect(
      fs.readFileSync(path.join(cwd, '.workspacer/skills/feedface/spawn-agent/SKILL.md'), 'utf8'),
    ).toBe('edited');
    expect(fs.existsSync(path.join(cwd, '.workspacer/brief.md'))).toBe(true);
  });

  it('removes Workspacer-owned personal manager skills, of any version, and nothing else', () => {
    const home = scratch();
    const old = (name: string) =>
      `---\nname: ${name}\ndescription: An older build. Only useful inside a Workspacer Fleet Manager session.\n---\n\nold\n`;
    write(path.join(home, '.claude/skills/standup/SKILL.md'), old('standup'));
    write(path.join(home, '.codex/skills/handoff/SKILL.md'), old('handoff'));
    write(
      path.join(home, '.claude/skills/checkpoint/SKILL.md'),
      '---\nname: checkpoint\n---\nmine\n',
    );
    write(path.join(home, '.codex/skills/standup/SKILL.md'), old('standup'));
    write(path.join(home, '.codex/skills/standup/notes.md'), 'user addition');
    prepareAgentSkills('copilot', scratch(), { home });
    expect(fs.existsSync(path.join(home, '.claude/skills/standup'))).toBe(false);
    expect(fs.existsSync(path.join(home, '.codex/skills/handoff'))).toBe(false);
    expect(fs.existsSync(path.join(home, '.claude/skills/checkpoint/SKILL.md'))).toBe(true);
    expect(fs.existsSync(path.join(home, '.codex/skills/standup/SKILL.md'))).toBe(true);
  });

  it('never follows a symlinked legacy parent', () => {
    const home = scratch();
    const cwd = scratch();
    const outside = scratch();
    const body = files['workspacer/skills/spawn-agent/SKILL.md'];
    write(path.join(outside, 'skills/spawn-agent/SKILL.md'), body);
    fs.symlinkSync(outside, path.join(cwd, '.agents'));
    prepareAgentSkills('codex', cwd, { home });
    expect(fs.readFileSync(path.join(outside, 'skills/spawn-agent/SKILL.md'), 'utf8')).toBe(body);
  });
});
