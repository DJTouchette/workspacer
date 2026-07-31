/**
 * The markdown preview pane is the unbounded path into the markdown renderer:
 * readFile hands it up to 5 MB and one click on a file link rendered all of it,
 * which took the app down hard enough to need a force quit. It renders a head
 * and says so.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';
import React from 'react';
import { render, screen, waitFor } from '@testing-library/react';
import MarkdownPreviewPane from '../../src/panes/MarkdownPreviewPane';

let readFile: ReturnType<typeof vi.fn>;

beforeEach(() => {
  readFile = vi.fn();
  (window as any).electronAPI = { ...(window as any).electronAPI, readFile };
});

describe('MarkdownPreviewPane size cap', () => {
  it('renders a huge file truncated, with a pointer to the editor', async () => {
    readFile.mockResolvedValue({ contents: `# Head\n\n${'word '.repeat(300_000)}` });
    const t0 = performance.now();
    render(<MarkdownPreviewPane previewPath="/repo/BIG.md" />);
    await waitFor(() => expect(screen.getByText(/Preview truncated/)).toBeTruthy());
    const took = performance.now() - t0;
    expect(screen.getByText('Head')).toBeTruthy();
    expect(took, `render took ${Math.round(took)}ms`).toBeLessThan(3000);
  });

  it('renders a normal file whole, with no truncation notice', async () => {
    readFile.mockResolvedValue({ contents: '# Title\n\nbody text' });
    render(<MarkdownPreviewPane previewPath="/repo/NOTES.md" />);
    await waitFor(() => expect(screen.getByText('Title')).toBeTruthy());
    expect(screen.queryByText(/Preview truncated/)).toBeNull();
  });
});
