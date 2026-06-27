import { describe, it, expect } from 'vitest';
import { summarizeWork } from '../../src/components/claude/WorkCard';
import type { ToolCall } from '../../src/types/claudeSession';

const tc = (over: Partial<ToolCall>): ToolCall =>
  ({ id: 'x', name: 'Edit', status: 'completed', input: {}, ...over }) as ToolCall;

describe('summarizeWork — MultiEdit line counting', () => {
  it('counts added/removed lines from a MultiEdit edits array', () => {
    // MultiEdit input has no top-level old_string/new_string — the edits live
    // in an `edits` array of { old_string, new_string }.
    const call = tc({
      name: 'MultiEdit',
      input: {
        file_path: '/a.ts',
        edits: [
          { old_string: 'one\ntwo', new_string: 'ONE\nTWO\nTHREE' }, // -2 +3
          { old_string: 'x', new_string: 'y\nz' }, // -1 +2
        ],
      },
    });
    const s = summarizeWork([call]);
    expect(s.editedFiles).toEqual(['/a.ts']);
    expect(s.removed).toBe(3);
    expect(s.added).toBe(5);
  });

  it('still counts a plain Edit from top-level old/new_string', () => {
    const call = tc({
      name: 'Edit',
      input: { file_path: '/b.ts', old_string: 'a\nb', new_string: 'c' },
    });
    const s = summarizeWork([call]);
    expect(s.removed).toBe(2);
    expect(s.added).toBe(1);
  });
});
