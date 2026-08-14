/**
 * Skills, as the chat UI and the Context pane show them.
 *
 * A live `system/init` frame reports 18 skills as PLAIN NAMES, and only two of
 * them are files under `~/.claude/skills` — the rest are compiled into the CLI
 * binary or shipped by a plugin. Everything here pins the consequences of that:
 * a skill invocation is not an anonymous "other" tool call, and a skill with no
 * file is labelled rather than left blank.
 */
import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import React from 'react';
import { summarizeWork } from '../../src/components/claude/WorkCard';
import { SkillCard, isSkillCall, skillCallName } from '../../src/components/claude/SkillCard';
import { SkillInventoryProvider } from '../../src/contexts/SkillInventoryContext';
import { byOrigin, BUILTIN_SOURCE } from '../../src/panes/ContextPane';
import { formatToolSummary } from '../../src/components/claude-shared';
import type { ToolCall, ContextItemInfo } from '../../src/types/claudeSession';

const skillCall = (input: Record<string, unknown>, id = 't1'): ToolCall =>
  ({ id, name: 'Skill', input, status: 'complete' }) as unknown as ToolCall;

const item = (name: string, extra: Partial<ContextItemInfo> = {}): ContextItemInfo => ({
  name,
  ...extra,
});

describe('a Skill tool call is recognised as one', () => {
  it('reads the skill name from `skill`, not "the first string in the input"', () => {
    // The generic fallback took the first string value, so a call carrying
    // `args` rendered the user's argument text where the skill name belongs.
    const tc = skillCall({ args: 'chart the revenue', skill: 'dataviz' });
    expect(isSkillCall(tc)).toBe(true);
    expect(skillCallName(tc)).toBe('dataviz');
    expect(formatToolSummary(tc).call).toBe('Skill(dataviz)');
  });

  it('counts skills in the work summary instead of filing them under "other"', () => {
    const summary = summarizeWork([
      skillCall({ skill: 'dataviz' }, 'a'),
      skillCall({ skill: 'verify' }, 'b'),
    ]);
    expect(summary.text).toBe('2 skills');
    expect(summary.text).not.toContain('other');
  });
});

describe('SkillCard shows what the session knows about the skill', () => {
  const renderCard = (tc: ToolCall, skills: ContextItemInfo[]) =>
    render(
      <SkillInventoryProvider skills={skills}>
        <SkillCard tc={tc} />
      </SkillInventoryProvider>,
    );

  it('renders the description and origin from the session inventory', () => {
    const { container } = renderCard(skillCall({ skill: 'omarchy' }), [
      item('omarchy', { description: 'Customize the Linux desktop.', source: 'user' }),
    ]);
    // The name sits between literal "Skill(" and ")" text nodes, so match the
    // rendered line rather than a single text node.
    expect(container.textContent).toContain('Skill(omarchy)');
    expect(screen.getByText('Customize the Linux desktop.')).toBeTruthy();
    expect(screen.getByText('user')).toBeTruthy();
  });

  it('labels a built-in rather than leaving the row bare', () => {
    renderCard(skillCall({ skill: 'dataviz' }), [item('dataviz', { source: BUILTIN_SOURCE })]);
    expect(screen.getByText(BUILTIN_SOURCE)).toBeTruthy();
  });

  it('degrades to the bare name when there is no inventory (PTY sessions)', () => {
    const { container } = render(<SkillCard tc={skillCall({ skill: 'dataviz' })} />);
    expect(container.textContent).toContain('Skill(dataviz)');
  });

  it('shows the arguments the skill was invoked with', () => {
    renderCard(skillCall({ skill: 'run', args: '--fast' }), [item('run')]);
    expect(screen.getByText('--fast')).toBeTruthy();
  });
});

describe('the Context pane orders skills by origin', () => {
  it('puts the assets the user owns ahead of the ones they cannot edit', () => {
    // The frame's own order is the CLI's load order, which buries the two
    // skills a user can actually edit among sixteen built-ins.
    const sorted = [
      item('zeta-builtin', { source: BUILTIN_SOURCE }),
      item('a-plugin', { source: 'code-review' }),
      item('m-user', { source: 'user' }),
      item('z-project', { source: 'project' }),
      item('a-builtin', { source: BUILTIN_SOURCE }),
    ].sort(byOrigin);

    expect(sorted.map((s) => s.name)).toEqual([
      'z-project',
      'm-user',
      'a-plugin',
      'a-builtin',
      'zeta-builtin',
    ]);
  });
});
