import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor, fireEvent } from '@testing-library/react';
import React from 'react';
import { ConversationMessage } from '../../src/components/claude/ConversationMessage';
import { __clearPreviewCache } from '../../src/components/claude/imagePreviews';
import { BROWSER_OPEN_EVENT } from '../../src/lib/browserBus';

/**
 * Images in the transcript. The composer already showed a thumbnail before you
 * sent; this is the same picture surviving the send instead of collapsing into
 * a `[Image: /path]` marker.
 *
 * The load path is asynchronous and allowed to fail (missing file, too big,
 * undecodable), so the contract is: a tile appears only for a path that really
 * decoded, and the message reads correctly either way.
 */

const PNG = 'data:image/png;base64,iVBORw0KGgo=';
const readImagePreview = vi.fn();

beforeEach(() => {
  __clearPreviewCache();
  readImagePreview.mockReset();
  readImagePreview.mockResolvedValue({ dataUrl: PNG });
  (window.electronAPI as any).readImagePreview = readImagePreview;
});

describe('attached images in a user message', () => {
  it('shows the picture and drops the marker from the text', async () => {
    render(
      <ConversationMessage
        turn={{
          role: 'user',
          content: '[Image: /home/me/shot.png] what is wrong with this layout?',
          timestamp: 1,
        }}
      />,
    );
    expect(await screen.findByAltText('shot.png')).toHaveAttribute('src', PNG);
    expect(screen.getByText('what is wrong with this layout?')).toBeInTheDocument();
    // The bookkeeping the agent needed is not something you should have to read.
    expect(screen.queryByText(/\[Image:/)).toBeNull();
  });

  it('renders an image-only message as just the image', async () => {
    render(
      <ConversationMessage
        turn={{ role: 'user', content: '[Image: /home/me/shot.png]', timestamp: 1 }}
      />,
    );
    expect(await screen.findByAltText('shot.png')).toBeInTheDocument();
    // No "(empty)" placeholder under a bubble whose content IS the picture.
    expect(screen.queryByText('(empty)')).toBeNull();
  });

  it('shows one tile per attachment', async () => {
    render(
      <ConversationMessage
        turn={{ role: 'user', content: '[Image: /a/one.png] [Image: /a/two.png] compare' }}
      />,
    );
    await waitFor(() => expect(screen.getAllByRole('img')).toHaveLength(2));
  });

  it('falls back to the marker text when the file cannot be previewed', async () => {
    readImagePreview.mockRejectedValue(new Error('gone'));
    render(
      <ConversationMessage
        turn={{ role: 'user', content: '[Image: /a/missing.png] look at this' }}
      />,
    );
    await waitFor(() => expect(readImagePreview).toHaveBeenCalled());
    expect(screen.queryByRole('img')).toBeNull();
    // The message still reads — the text it accompanied is there.
    expect(screen.getByText('look at this')).toBeInTheDocument();
  });

  it('shows what was attached when no tile can render, never an empty bubble', async () => {
    // A .tiff (or a deleted file, or one over the size cap): the marker was
    // stripped on extension alone, before anything tried to decode it.
    readImagePreview.mockRejectedValue(new Error('cannot decode tiff'));
    render(<ConversationMessage turn={{ role: 'user', content: '[Image: /p/scan.tiff]' }} />);
    expect(await screen.findByText('[Image: /p/scan.tiff]')).toBeInTheDocument();
  });

  it('does not flash the marker while the preview is still loading', async () => {
    let resolve!: (v: { dataUrl: string }) => void;
    readImagePreview.mockReturnValue(new Promise((r) => (resolve = r)));
    render(<ConversationMessage turn={{ role: 'user', content: '[Image: /a/slow.png]' }} />);
    // In flight: neither a tile nor the fallback — settling is what decides.
    expect(screen.queryByText('[Image: /a/slow.png]')).toBeNull();
    expect(screen.queryByRole('img')).toBeNull();
    resolve({ dataUrl: PNG });
    expect(await screen.findByAltText('slow.png')).toBeInTheDocument();
    expect(screen.queryByText('[Image: /a/slow.png]')).toBeNull();
  });

  it('opens the image in a browser pane when clicked', async () => {
    const seen: any[] = [];
    const onOpen = (e: Event) => seen.push((e as CustomEvent).detail);
    window.addEventListener(BROWSER_OPEN_EVENT, onOpen);
    try {
      render(
        <ConversationMessage turn={{ role: 'user', content: '[Image: /home/me/shot.png]' }} />,
      );
      fireEvent.click(await screen.findByAltText('shot.png'));
      expect(seen).toHaveLength(1);
      expect(seen[0].url).toContain('shot.png');
      expect(seen[0].title).toBe('shot.png');
    } finally {
      window.removeEventListener(BROWSER_OPEN_EVENT, onOpen);
    }
  });
});

describe('images an agent mentions', () => {
  it('thumbnails a screenshot the agent says it wrote, keeping the path in the prose', async () => {
    render(
      <ConversationMessage
        turn={{ role: 'assistant', content: 'Saved the chart to /tmp/chart.png — take a look.' }}
      />,
    );
    expect(await screen.findByAltText('chart.png')).toBeInTheDocument();
    // Unlike an attachment marker, a mention is part of the sentence: it stays.
    expect(screen.getByText(/take a look/)).toBeInTheDocument();
  });

  it('leaves a message with no images alone', () => {
    render(<ConversationMessage turn={{ role: 'assistant', content: 'Edited src/app.ts' }} />);
    expect(screen.queryByRole('img')).toBeNull();
    expect(readImagePreview).not.toHaveBeenCalled();
  });

  it('resolves a relative mention against the session cwd', async () => {
    render(
      <ConversationMessage
        turn={{ role: 'user', content: '[Image: shots/one.png] here' }}
        cwd="/repo"
      />,
    );
    await waitFor(() => expect(readImagePreview).toHaveBeenCalledWith('/repo/shots/one.png'));
  });
});
