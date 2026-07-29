/**
 * Pasted-screenshot spilling. A screenshot on the clipboard has no file behind
 * it, so there's no path to attach — main writes one. The cases worth pinning:
 * an empty clipboard must return null (so the paste falls through to text
 * handling rather than attaching a 0-byte PNG), and two pastes in the same
 * millisecond must not land on the same filename.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';
import * as fs from 'fs';

const { readImage } = vi.hoisted(() => ({ readImage: vi.fn() }));

vi.mock('electron', () => ({ clipboard: { readImage: () => readImage() } }));

const { savePastedImage, pastedImageDir } = await import('./clipboardImage');

const PNG = Buffer.from('fake-png-bytes');

function image(opts: { empty?: boolean } = {}) {
  return {
    isEmpty: () => opts.empty ?? false,
    toPNG: () => PNG,
    getSize: () => ({ width: 1440, height: 900 }),
  };
}

beforeEach(() => {
  readImage.mockReset();
  fs.rmSync(pastedImageDir(), { recursive: true, force: true });
});

describe('savePastedImage', () => {
  it('returns null when the clipboard holds no image', () => {
    readImage.mockReturnValue(image({ empty: true }));
    expect(savePastedImage()).toBeNull();
  });

  it('writes the image and reports its path and size', () => {
    readImage.mockReturnValue(image());
    const saved = savePastedImage();

    expect(saved).not.toBeNull();
    expect(saved!.path.endsWith('.png')).toBe(true);
    expect(fs.readFileSync(saved!.path)).toEqual(PNG);
    expect(saved).toMatchObject({ width: 1440, height: 900 });
  });

  it('never collides, even within one millisecond', () => {
    readImage.mockReturnValue(image());
    const paths = new Set([savePastedImage()!.path, savePastedImage()!.path]);
    expect(paths.size).toBe(2);
  });
});
