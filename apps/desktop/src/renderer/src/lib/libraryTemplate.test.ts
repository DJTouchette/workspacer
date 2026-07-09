import { describe, it, expect } from 'vitest';
import { renderItemText } from './libraryTemplate';

describe('renderItemText — skill framing by scope', () => {
  it('frames a workspacer (project/global) skill with a header + its body — it is copy-paste text', () => {
    const out = renderItemText({
      scope: 'project',
      kind: 'skill',
      title: 'Careful refactor',
      description: 'Small steps.',
      body: 'Do the thing.',
    });
    expect(out).toContain('# Skill: Careful refactor');
    expect(out).toContain('Small steps.');
    expect(out).toContain('Do the thing.');
  });

  it('references a claude-scope skill by name instead of pasting its body (Claude already has the real skill)', () => {
    const body = 'A big SKILL.md body Claude already knows how to run.';
    const out = renderItemText({
      scope: 'claude',
      kind: 'skill',
      title: 'pdf-filler',
      description: 'Fills PDFs.',
      body,
    });
    // Invocation reference, not the body dump.
    expect(out).toContain('pdf-filler');
    expect(out).not.toContain(body);
    expect(out).not.toContain('# Skill:');
  });

  it('inserts a prompt verbatim regardless of scope', () => {
    expect(renderItemText({ scope: 'global', kind: 'prompt', title: 'P', body: 'raw text' })).toBe(
      'raw text',
    );
  });
});
