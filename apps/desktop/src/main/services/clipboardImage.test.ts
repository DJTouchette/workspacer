/**
 * Pasted-screenshot spilling. A screenshot on the clipboard has no file behind
 * it, so there's no path to attach — main writes one. The cases worth pinning:
 * an empty clipboard must return null (so the paste falls through to text
 * handling rather than attaching a 0-byte PNG), and two pastes in the same
 * millisecond must not land on the same filename.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import * as fs from 'fs';

const { readImage } = vi.hoisted(() => ({ readImage: vi.fn() }));

vi.mock('electron', () => ({ clipboard: { readImage: () => readImage() } }));

const { savePastedImage } = await import('./clipboardImage');

const PNG = Buffer.from('fake-png-bytes');

function image(opts: { empty?: boolean } = {}) {
  return {
    isEmpty: () => opts.empty ?? false,
    toPNG: () => PNG,
    getSize: () => ({ width: 1440, height: 900 }),
  };
}

// Only ever remove the files this suite created. The spill directory is shared
// with the running app (it resolves under os.tmpdir()), so clearing it wholesale
// would delete screenshots a live composer had just pasted.
const written: string[] = [];
const spill = () => {
  const saved = savePastedImage();
  if (saved) written.push(saved.path);
  return saved;
};

beforeEach(() => {
  readImage.mockReset();
});

afterEach(() => {
  while (written.length) fs.rmSync(written.pop()!, { force: true });
});

describe('savePastedImage', () => {
  it('returns null when the clipboard holds no image', () => {
    readImage.mockReturnValue(image({ empty: true }));
    expect(spill()).toBeNull();
  });

  it('writes the image and reports its path and size', () => {
    readImage.mockReturnValue(image());
    const saved = spill();

    expect(saved).not.toBeNull();
    expect(saved!.path.endsWith('.png')).toBe(true);
    expect(fs.readFileSync(saved!.path)).toEqual(PNG);
    expect(saved).toMatchObject({ width: 1440, height: 900 });
  });

  it('never collides, even within one millisecond', () => {
    readImage.mockReturnValue(image());
    const paths = new Set([spill()!.path, spill()!.path]);
    expect(paths.size).toBe(2);
  });
});
