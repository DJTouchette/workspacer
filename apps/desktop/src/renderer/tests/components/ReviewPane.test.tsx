import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import React from 'react';
import ReviewPane from '../../src/panes/ReviewPane';

const api = window.electronAPI as unknown as Record<string, ReturnType<typeof vi.fn>>;

const dirtyStatus = {
  branch: 'main',
  upstream: 'origin/main',
  ahead: 0,
  behind: 0,
  files: [{ path: 'app.ts', staged: ' ', unstaged: 'M' }],
};

const stagedStatus = {
  branch: 'main',
  upstream: 'origin/main',
  ahead: 0,
  behind: 0,
  files: [{ path: 'app.ts', staged: 'M', unstaged: ' ' }],
};

const committedStatus = {
  branch: 'main',
  upstream: 'origin/main',
  ahead: 1,
  behind: 0,
  files: [],
};

const pushedStatus = {
  branch: 'main',
  upstream: 'origin/main',
  ahead: 0,
  behind: 0,
  files: [],
};

beforeEach(() => {
  api.gitStatus = vi
    .fn()
    .mockResolvedValueOnce(dirtyStatus)
    .mockResolvedValueOnce(stagedStatus)
    .mockResolvedValueOnce(committedStatus)
    .mockResolvedValueOnce(pushedStatus);
  api.gitNumstat = vi.fn().mockResolvedValue([{ path: 'app.ts', added: 3, deleted: 1 }]);
  api.gitDiff = vi
    .fn()
    .mockResolvedValue(
      [
        'diff --git a/app.ts b/app.ts',
        '--- a/app.ts',
        '+++ b/app.ts',
        '@@ -1 +1 @@',
        '-old',
        '+new',
      ].join('\n'),
    );
  api.gitStage = vi.fn().mockResolvedValue('');
  api.gitUnstage = vi.fn().mockResolvedValue('');
  api.gitCommit = vi.fn().mockResolvedValue('[main abc123] Close review loop');
  api.gitPush = vi.fn().mockResolvedValue('To origin');
});

describe('ReviewPane completion loop', () => {
  it('surfaces commit and push success, then returns to the owning agent', async () => {
    const onReturnToAgent = vi.fn();
    render(
      <ReviewPane
        paneId="review-1"
        title="Review"
        isActive
        cwd="/repo"
        onReturnToAgent={onReturnToAgent}
      />,
    );

    expect(await screen.findByText('app.ts')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: /^Stage$/ }));
    await waitFor(() => expect(api.gitStage).toHaveBeenCalledWith('/repo', 'app.ts'));

    fireEvent.change(await screen.findByPlaceholderText(/commit message/i), {
      target: { value: 'Close review loop' },
    });
    fireEvent.click(screen.getByRole('button', { name: /Commit 1 file/i }));

    expect(await screen.findByText('Committed staged changes')).toBeInTheDocument();
    expect(screen.getByText('Push 1 commit to origin/main.')).toBeInTheDocument();
    expect(api.gitCommit).toHaveBeenCalledWith('/repo', 'Close review loop');

    fireEvent.click(screen.getByRole('button', { name: /^Push$/ }));

    expect(await screen.findByText('Review complete')).toBeInTheDocument();
    expect(screen.getByText('Changes are committed and pushed.')).toBeInTheDocument();
    expect(api.gitPush).toHaveBeenCalledWith('/repo');

    fireEvent.click(screen.getByRole('button', { name: /Back to agent/i }));
    expect(onReturnToAgent).toHaveBeenCalledTimes(1);
  });
});
