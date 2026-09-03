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

/**
 * The TOCTOU re-check: a pane opened from a checked `file:` URL carries the
 * canonical path main verified, and must hand it to `readFile` so a fresh
 * canonicalization at read time can be compared against it (see ipc.ts's
 * `file:read` handler). FileLink's own preview path carries no such check, so
 * it must NOT get one manufactured for it here.
 */
describe('MarkdownPreviewPane: TOCTOU re-check plumbing', () => {
  it('passes previewCanonicalPath through to the reader unchanged', async () => {
    readFile.mockResolvedValue({ contents: '# Title' });
    render(
      <MarkdownPreviewPane previewPath="/repo/NOTES.md" previewCanonicalPath="/repo/NOTES.md" />,
    );
    await waitFor(() => expect(readFile).toHaveBeenCalledWith('/repo/NOTES.md', '/repo/NOTES.md'));
  });

  it('reads with no expected path when the pane carries none (FileLink)', async () => {
    readFile.mockResolvedValue({ contents: '# Title' });
    render(<MarkdownPreviewPane previewPath="/repo/NOTES.md" />);
    await waitFor(() => expect(readFile).toHaveBeenCalledWith('/repo/NOTES.md', undefined));
  });
});
