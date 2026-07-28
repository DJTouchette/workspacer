import { describe, it, expect } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import React from 'react';
import { WorkCard } from '../../src/components/claude/WorkCard';
import { ToolTraceCard } from '../../src/components/claude/ToolTraceCard';
import type { ToolCall } from '../../src/types/claudeSession';

/**
 * `claude.showFileReads` gates the inline body of a Read tool call in the work
 * log. Both work-log styles (cards + trace) take the same `showReads` prop and
 * must hide the file contents identically — and the trace card must not leak
 * the same bytes back through its generic response fallback.
 */

const SECRET_LINE = 'const apiKey = "hunter2";';

const readCall = (): ToolCall =>
  ({
    id: 'tc-read',
    name: 'Read',
    status: 'completed',
    input: { file_path: '/repo/keys.ts' },
    response: `     1\t${SECRET_LINE}\n     2\texport {};\n`,
    startedAt: 1000,
    completedAt: 1200,
  }) as ToolCall;

describe('work log — file-read contents are gated by showReads', () => {
  it('WorkCard renders the read body by default and drops it when showReads is false', () => {
    const { unmount } = render(<WorkCard toolCalls={[readCall()]} isLast cwd="/repo" />);
    expect(screen.getByText(SECRET_LINE)).toBeTruthy();
    unmount();

    render(<WorkCard toolCalls={[readCall()]} isLast cwd="/repo" showReads={false} />);
    expect(screen.queryByText(SECRET_LINE)).toBeNull();
    // The call itself is still logged — only its body is hidden.
    expect(screen.getAllByText(/keys\.ts/).length).toBeGreaterThan(0);
  });

  it('ToolTraceCard renders the read body by default and drops it when showReads is false', () => {
    // Click the tool chip, not the target: the target is a FileLink and stops
    // propagation, so it opens the file instead of expanding the row.
    const openRow = () => fireEvent.click(screen.getByText('Read'));

    const { unmount } = render(<ToolTraceCard toolCalls={[readCall()]} isLast cwd="/repo" />);
    openRow();
    expect(screen.getByText(SECRET_LINE)).toBeTruthy();
    unmount();

    render(<ToolTraceCard toolCalls={[readCall()]} isLast cwd="/repo" showReads={false} />);
    openRow();
    // Neither ReadView nor the generic input/response `pre` fallback may show it.
    expect(screen.queryByText(new RegExp(SECRET_LINE.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')))).toBe(
      null,
    );
    // The row still expands to what was read.
    expect(screen.getAllByText(/keys\.ts/).length).toBeGreaterThan(0);
  });
});
