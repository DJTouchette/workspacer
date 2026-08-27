/**
 * The strict host-side renderer for dispatch templates (lib/dispatchTemplate).
 *
 * The load-bearing behaviour is the HARD ERROR: placeholders are required by
 * default and an unfilled one refuses the render naming the missing param —
 * never a silent default, which is what the renderer's applyTemplate does for
 * its interactive form fields and exactly what a dispatch must not do (a
 * rendered template reads finished, so a silent default dispatches boilerplate
 * without the task-specific reasoning only the manager can write).
 */
import { describe, it, expect } from 'vitest';
import { renderDispatchTemplate, dispatchTemplateParams } from './dispatchTemplate';

describe('renderDispatchTemplate — happy path', () => {
  it('fills required placeholders from params and {{cwd}} from context', () => {
    const out = renderDispatchTemplate(
      'SHIP TASK in {{cwd}}.\n\n{{task}}\n\nReport when done.',
      { task: 'Fix the off-by-one in parse()' },
      { cwd: '/home/u/proj' },
    );
    expect(out).toBe(
      'SHIP TASK in /home/u/proj.\n\nFix the off-by-one in parse()\n\nReport when done.',
    );
  });

  it('tolerates the renderer prompt-var spelling ({{?name}}) with the same params', () => {
    expect(renderDispatchTemplate('do {{?task}}', { task: 'x' })).toBe('do x');
  });

  it('a param value wins over an optional placeholder default', () => {
    const out = renderDispatchTemplate(
      'Deliver: {{delivery:open a PR}}',
      { delivery: 'merge locally on a branch' },
      {},
    );
    expect(out).toBe('Deliver: merge locally on a branch');
  });

  it('an unfilled OPTIONAL placeholder renders its explicit default', () => {
    expect(renderDispatchTemplate('Deliver: {{delivery:open a PR}}', {})).toBe(
      'Deliver: open a PR',
    );
  });

  it('a param named cwd overrides the auto var', () => {
    expect(renderDispatchTemplate('in {{cwd}}', { cwd: '/elsewhere' }, { cwd: '/proj' })).toBe(
      'in /elsewhere',
    );
  });
});

describe('renderDispatchTemplate — the hard rule', () => {
  it('an unfilled REQUIRED placeholder is an error naming the param, never a default', () => {
    expect(() => renderDispatchTemplate('do {{task}} now', {})).toThrow(/\{\{task\}\}/);
  });

  it('a blank value is the same dodge as omitting it', () => {
    expect(() => renderDispatchTemplate('do {{task}}', { task: '   ' })).toThrow(/\{\{task\}\}/);
    expect(() => renderDispatchTemplate('do {{task}}', { task: '' })).toThrow(/\{\{task\}\}/);
  });

  it('names the FIRST missing param of several, and mentions the optional syntax', () => {
    expect(() => renderDispatchTemplate('{{symptom}} vs {{explanationA}}', {})).toThrow(
      /\{\{symptom\}\}.*optional/s,
    );
  });

  it('a param naming no placeholder is refused too (typo guard)', () => {
    expect(() => renderDispatchTemplate('do {{task}}', { task: 'x', tsak: 'y' })).toThrow(
      /unknown templateParams "tsak"/,
    );
  });

  it('{{cwd}} is an auto var, not a required param', () => {
    expect(renderDispatchTemplate('in {{cwd}}', {}, {})).toBe('in ');
  });
});

describe('dispatchTemplateParams', () => {
  it('lists distinct placeholder names in first-seen order, auto vars excluded', () => {
    const params = dispatchTemplateParams(
      'in {{cwd}}: {{task}} then {{delivery:open a PR}} and {{task}} again',
    );
    expect(params.map((p) => p.name)).toEqual(['task', 'delivery']);
    expect(params[0].defaultValue).toBeUndefined();
    expect(params[1].defaultValue).toBe('open a PR');
  });
});
