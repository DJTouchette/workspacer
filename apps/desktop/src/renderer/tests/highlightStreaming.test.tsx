/**
 * Regression tests for the streaming-code-fence tokenizer cost.
 *
 * `parseMarkdownBlocks` emits a CodeBlock for an UNTERMINATED fence, so a fence
 * an agent is still writing re-renders on every snapshot flush (~16ms) with a
 * slightly longer string. `useHighlight` was keyed on the whole string and
 * `tokenize` cached the grammar but not the result, so each flush pushed the
 * entire growing prefix through the TextMate engine again — synchronously, on
 * the main thread. The work for a fence of L lines was quadratic in L.
 *
 * These assert the two properties that keep it linear-ish, both of which are
 * invisible to every other test in the suite: a regression here is not a wrong
 * pixel, it is just slow.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, waitFor, act } from '@testing-library/react';
import React from 'react';

// Count real tokenizer entries. The mock stands in for shiki's blocking
// codeToTokensBase; what matters is HOW MANY TIMES it is reached and with WHAT.
const calls: string[] = [];
vi.mock('../src/lib/diff/highlight', async () => {
  const actual = await vi.importActual<typeof import('../src/lib/diff/highlight')>(
    '../src/lib/diff/highlight',
  );
  return {
    ...actual,
    MAX_HIGHLIGHT_LINE_LENGTH: 1000,
    tokenize: vi.fn(async (code: string) => {
      calls.push(code);
      return code.split('\n').map((l) => [{ text: l, color: '#fff' }]);
    }),
  };
});

import { useHighlight } from '../src/components/claude/highlight';

/** Renders useHighlight and re-renders it with each successive `code`. */
const Probe: React.FC<{ code: string }> = ({ code }) => {
  const tokens = useHighlight(code, 'ts');
  return <div data-testid="out">{tokens ? tokens.length : 'null'}</div>;
};

beforeEach(() => {
  calls.length = 0;
  vi.useFakeTimers({ shouldAdvanceTime: true });
});
afterEach(() => {
  vi.useRealTimers();
});

describe('useHighlight on a growing code fence', () => {
  it('tokenizes only settled lines while text is still arriving', async () => {
    const { rerender } = render(<Probe code={'const a = 1\nconst b = 2\nconst c' as string} />);
    await waitFor(() => expect(calls.length).toBe(1));
    calls.length = 0;

    // One more token lands on the unfinished last line.
    rerender(<Probe code={'const a = 1\nconst b = 2\nconst c ='} />);
    await act(async () => {
      vi.advanceTimersByTime(500);
    });

    await waitFor(() => expect(calls.length).toBeGreaterThan(0));
    // The partial final line must NOT be in what we tokenized — it is the part
    // that changes every frame, and including it is what made the cost
    // quadratic. Everything up to the last newline is fair game.
    expect(calls.every((c) => !c.includes('const c ='))).toBe(true);
  });

  it('debounces while growing — a burst of appends is not a burst of tokenizations', async () => {
    const base = 'line one\nline two\n';
    const { rerender } = render(<Probe code={base} />);
    await waitFor(() => expect(calls.length).toBe(1));
    calls.length = 0;

    // Ten flushes in rapid succession, as a streaming turn produces.
    for (let i = 0; i < 10; i++) {
      rerender(<Probe code={`${base}more ${'x'.repeat(i)}`} />);
      await act(async () => {
        vi.advanceTimersByTime(20);
      });
    }
    // Before the fix this was ~10 full passes over the whole prefix.
    expect(calls.length).toBeLessThan(5);
  });

  it('tokenizes settled text immediately — no debounce on first paint', async () => {
    render(<Probe code={'const x = 1\nconst y = 2\n'} />);
    // Not advancing timers at all: a block that is not growing must not wait.
    await waitFor(() => expect(calls.length).toBe(1));
    expect(calls[0]).toContain('const y = 2');
  });

  it('tokenizes the full text once the fence stops growing', async () => {
    const { rerender } = render(<Probe code={'a = 1\nb = '} />);
    await waitFor(() => expect(calls.length).toBe(1));

    rerender(<Probe code={'a = 1\nb = 2'} />);
    await act(async () => {
      vi.advanceTimersByTime(500);
    });
    // A different string that is NOT an extension of the last one (the fence
    // closed, or the turn was replaced) is tokenized whole — the tail must not
    // stay permanently unhighlighted.
    rerender(<Probe code={'totally different\ncontent here'} />);
    await waitFor(() => expect(calls.at(-1)).toContain('content here'));
  });
});
