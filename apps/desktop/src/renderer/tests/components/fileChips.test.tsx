import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor, fireEvent } from '@testing-library/react';
import React from 'react';
import { FileChips, __clearPreviewCache } from '../../src/components/claude/FileChips';
import type { AttachedFile } from '../../src/components/claude/fileAttachment';

/**
 * Composer attachments: image chips become thumbnails, everything else stays a
 * pill. The renderer can't read host paths itself (in dev it's served over
 * http, where file:// is blocked), so every preview comes from main via
 * readImagePreview — and a failure there has to degrade to the icon chip
 * rather than render a broken <img>.
 */

const readImagePreview = vi.fn();

beforeEach(() => {
  __clearPreviewCache();
  readImagePreview.mockReset();
  (window as unknown as { electronAPI: unknown }).electronAPI = { readImagePreview };
});

const image = (name: string): AttachedFile => ({ path: `/repo/${name}`, name, label: 'Image' });

describe('FileChips', () => {
  it('renders a thumbnail once main returns a preview', async () => {
    readImagePreview.mockResolvedValue({
      path: '/repo/shot.png',
      dataUrl: 'data:image/png;base64,AAA',
      width: 800,
      height: 600,
      size: 1234,
    });
    render(<FileChips files={[image('shot.png')]} onRemove={vi.fn()} />);

    const img = await screen.findByAltText('shot.png');
    expect(img).toHaveAttribute('src', 'data:image/png;base64,AAA');
    expect(readImagePreview).toHaveBeenCalledWith('/repo/shot.png');
  });

  it('falls back to the icon chip when the preview fails', async () => {
    readImagePreview.mockRejectedValue(new Error('ENOENT'));
    render(<FileChips files={[image('gone.png')]} onRemove={vi.fn()} />);

    await waitFor(() => expect(readImagePreview).toHaveBeenCalled());
    expect(screen.queryByAltText('gone.png')).not.toBeInTheDocument();
    expect(screen.getByText('gone.png')).toBeInTheDocument();
  });

  it('never asks for a preview of a non-image attachment', async () => {
    render(
      <FileChips
        files={[{ path: '/repo/spec.pdf', name: 'spec.pdf', label: 'PDF' }]}
        onRemove={vi.fn()}
      />,
    );
    expect(screen.getByText('spec.pdf')).toBeInTheDocument();
    expect(readImagePreview).not.toHaveBeenCalled();
  });

  it('decodes each path once, however often the parent re-renders', async () => {
    readImagePreview.mockResolvedValue({
      path: '/repo/a.png',
      dataUrl: 'data:image/png;base64,AAA',
      width: 10,
      height: 10,
      size: 10,
    });
    const files = [image('a.png')];
    const { rerender } = render(<FileChips files={files} onRemove={vi.fn()} />);
    await screen.findByAltText('a.png');
    // A fresh array with the same paths — the effect must not re-fire.
    rerender(<FileChips files={[image('a.png')]} onRemove={vi.fn()} />);
    await waitFor(() => expect(readImagePreview).toHaveBeenCalledTimes(1));
  });

  it('removes by index from both chip shapes', async () => {
    readImagePreview.mockResolvedValue({
      path: '/repo/b.png',
      dataUrl: 'data:image/png;base64,BBB',
      width: 10,
      height: 10,
      size: 10,
    });
    const onRemove = vi.fn();
    render(
      <FileChips
        files={[{ path: '/repo/spec.pdf', name: 'spec.pdf', label: 'PDF' }, image('b.png')]}
        onRemove={onRemove}
      />,
    );
    await screen.findByAltText('b.png');

    fireEvent.click(screen.getByLabelText('Remove b.png'));
    expect(onRemove).toHaveBeenCalledWith(1);
    fireEvent.click(screen.getByLabelText('Remove spec.pdf'));
    expect(onRemove).toHaveBeenCalledWith(0);
  });
});
