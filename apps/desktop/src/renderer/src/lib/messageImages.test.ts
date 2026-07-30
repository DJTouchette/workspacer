import { describe, it, expect } from 'vitest';
import { extractImageAttachments, imagePathsInText, isImagePath } from './messageImages';

/**
 * The composer writes attachments into the message text as `[Image: /path]`
 * markers for the agent to read. The transcript shows the picture instead — but
 * only when it can: a marker for something unrenderable must stay as text
 * rather than silently deleting the evidence that a file was sent.
 */

describe('isImagePath', () => {
  it('accepts the raster formats the preview pipeline can decode', () => {
    for (const p of ['/a/b.png', '/a/b.JPG', '/a/b.jpeg', '/a/b.gif', '/a/b.webp', '/a/b.tiff']) {
      expect(isImagePath(p)).toBe(true);
    }
  });

  it('rejects everything else', () => {
    for (const p of ['/a/b.pdf', '/a/b.txt', '/a/png', '/a/b.png.bak', '']) {
      expect(isImagePath(p)).toBe(false);
    }
  });
});

describe('extractImageAttachments', () => {
  it('pulls the image out and leaves the message you typed', () => {
    const { text, paths } = extractImageAttachments(
      '[Image: /home/me/shot.png] what is wrong with this layout?',
    );
    expect(paths).toEqual(['/home/me/shot.png']);
    expect(text).toBe('what is wrong with this layout?');
  });

  it('handles several attachments in one message', () => {
    const { text, paths } = extractImageAttachments(
      '[Image: /a/one.png] [Image: /a/two.jpg] compare these',
    );
    expect(paths).toEqual(['/a/one.png', '/a/two.jpg']);
    expect(text).toBe('compare these');
  });

  it('leaves an image-only message with no text', () => {
    const { text, paths } = extractImageAttachments('[Image: /a/one.png]');
    expect(paths).toEqual(['/a/one.png']);
    expect(text).toBe('');
  });

  it('keeps File and PDF markers — there is nothing to show for them', () => {
    const { text, paths } = extractImageAttachments(
      '[PDF: /a/spec.pdf] [File: /a/notes.txt] read these',
    );
    expect(paths).toEqual([]);
    expect(text).toBe('[PDF: /a/spec.pdf] [File: /a/notes.txt] read these');
  });

  it('keeps an Image marker whose path it cannot render, rather than eating it', () => {
    const { text, paths } = extractImageAttachments('[Image: /a/thing.heic] look');
    expect(paths).toEqual([]);
    expect(text).toContain('[Image: /a/thing.heic]');
  });

  it('dedupes the same attachment mentioned twice', () => {
    const { paths } = extractImageAttachments('[Image: /a/one.png] [Image: /a/one.png] hi');
    expect(paths).toEqual(['/a/one.png']);
  });

  it('handles paths with spaces and Windows drives', () => {
    const { text, paths } = extractImageAttachments(
      '[Image: C:\\Users\\me\\my shot.png] check the toolbar',
    );
    expect(paths).toEqual(['C:\\Users\\me\\my shot.png']);
    expect(text).toBe('check the toolbar');
  });

  it('survives empty and undefined content', () => {
    expect(extractImageAttachments(undefined)).toEqual({ text: '', paths: [] });
    expect(extractImageAttachments('')).toEqual({ text: '', paths: [] });
  });
});

describe('imagePathsInText', () => {
  it('finds an image an agent says it wrote', () => {
    expect(imagePathsInText('Saved the chart to /tmp/chart.png — take a look.')).toEqual([
      '/tmp/chart.png',
    ]);
  });

  it('strips sentence punctuation off the end of a path', () => {
    expect(imagePathsInText('Wrote /tmp/a.png.')).toEqual(['/tmp/a.png']);
    expect(imagePathsInText('Wrote /tmp/a.png, then stopped')).toEqual(['/tmp/a.png']);
  });

  it('reads a path out of a code span', () => {
    expect(imagePathsInText('Saved to `/tmp/shot.png` for review')).toEqual(['/tmp/shot.png']);
  });

  it('ignores non-images and relative mentions it cannot resolve', () => {
    expect(imagePathsInText('Edited /src/app.ts and shot.png')).toEqual([]);
  });

  it('caps the strip so a frame dump does not become a contact sheet', () => {
    const many = Array.from({ length: 12 }, (_, i) => `/tmp/f${i}.png`).join(' ');
    expect(imagePathsInText(many)).toHaveLength(4);
    expect(imagePathsInText(many, 2)).toHaveLength(2);
  });

  it('dedupes a path mentioned twice', () => {
    expect(imagePathsInText('/tmp/a.png then again /tmp/a.png')).toEqual(['/tmp/a.png']);
  });

  it('survives empty and undefined content', () => {
    expect(imagePathsInText(undefined)).toEqual([]);
    expect(imagePathsInText('')).toEqual([]);
  });
});
