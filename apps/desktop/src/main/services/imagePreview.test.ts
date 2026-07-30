/**
 * Composer attachment thumbnails. The two things worth pinning: the extension
 * allow-list (this is reachable from the hub as `fs.readImage`, so it must not
 * become a base64-any-file primitive), and the fallback for formats Chromium
 * declines to decode — SVG in particular comes back as an *empty* nativeImage
 * rather than an error, which is easy to mistake for a valid thumbnail.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

const { createFromPath } = vi.hoisted(() => ({ createFromPath: vi.fn() }));

vi.mock('electron', () => ({
  nativeImage: { createFromPath: (p: string) => createFromPath(p) },
}));

const { readImagePreview, isPreviewableImage } = await import('./imagePreview');

/** A nativeImage stand-in that reports `size` and records resize calls. */
function fakeImage(size: { width: number; height: number }) {
  const resize = vi.fn(() => img);
  const img = {
    isEmpty: () => false,
    getSize: () => size,
    resize,
    toDataURL: () => 'data:image/png;base64,UE5H',
    toJPEG: () => Buffer.from('JPEG'),
  };
  return img;
}

const emptyImage = { isEmpty: () => true } as unknown as ReturnType<typeof fakeImage>;

let dir: string;
beforeEach(() => {
  createFromPath.mockReset();
  dir = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-thumb-'));
});

function write(name: string, contents: string | Buffer): string {
  const p = path.join(dir, name);
  fs.writeFileSync(p, contents);
  return p;
}

describe('readImagePreview', () => {
  it('refuses a non-image extension without reading the file', () => {
    const p = write('secrets.txt', 'hunter2');
    expect(() => readImagePreview(p)).toThrow(/not a previewable image/);
    expect(createFromPath).not.toHaveBeenCalled();
  });

  it('refuses a directory that happens to be named like an image', () => {
    const p = path.join(dir, 'shots.png');
    fs.mkdirSync(p);
    expect(() => readImagePreview(p)).toThrow(/not a regular file/);
  });

  it('downscales an oversized image to the thumbnail edge, preserving aspect', () => {
    const img = fakeImage({ width: 4000, height: 2000 });
    createFromPath.mockReturnValue(img);
    const p = write('wide.png', 'not-really-png');

    const preview = readImagePreview(p);

    expect(img.resize).toHaveBeenCalledWith({ width: 256, height: 128, quality: 'good' });
    expect(preview.dataUrl).toBe('data:image/png;base64,UE5H');
    // Reported size is the *source* size — the UI shows what the agent gets.
    expect(preview.width).toBe(4000);
    expect(preview.height).toBe(2000);
  });

  it('leaves an already-small image alone', () => {
    const img = fakeImage({ width: 48, height: 48 });
    createFromPath.mockReturnValue(img);
    readImagePreview(write('icon.png', 'x'));
    expect(img.resize).not.toHaveBeenCalled();
  });

  it('encodes photos as JPEG rather than PNG', () => {
    createFromPath.mockReturnValue(fakeImage({ width: 100, height: 100 }));
    const preview = readImagePreview(write('photo.jpg', 'x'));
    expect(preview.dataUrl).toBe(
      `data:image/jpeg;base64,${Buffer.from('JPEG').toString('base64')}`,
    );
  });

  it('inlines the original bytes when Chromium cannot decode the format (SVG)', () => {
    createFromPath.mockReturnValue(emptyImage);
    const svg = '<svg xmlns="http://www.w3.org/2000/svg"/>';
    const preview = readImagePreview(write('logo.svg', svg));
    expect(preview.dataUrl).toBe(
      `data:image/svg+xml;base64,${Buffer.from(svg).toString('base64')}`,
    );
    // No decode happened, so there is no intrinsic pixel size to report.
    expect(preview.width).toBe(0);
  });

  it('refuses a decode bomb on its header, before the decoder is handed the file', () => {
    // 20000x20000 of flat colour: tiny on disk, gigabytes decoded. The size cap
    // waves it through, so the pixel cap is the only thing that stops it.
    const header = Buffer.alloc(24);
    header.writeUInt32BE(0x89504e47, 0);
    header.writeUInt32BE(0x0d0a1a0a, 4);
    header.write('IHDR', 12, 'ascii');
    header.writeUInt32BE(20000, 16);
    header.writeUInt32BE(20000, 20);
    const p = write('bomb.png', header);

    expect(() => readImagePreview(p)).toThrow(/max .* pixels/);
    expect(createFromPath).not.toHaveBeenCalled();
  });

  it('allows a large but sane photo through', () => {
    const header = Buffer.alloc(24);
    header.writeUInt32BE(0x89504e47, 0);
    header.writeUInt32BE(0x0d0a1a0a, 4);
    header.write('IHDR', 12, 'ascii');
    header.writeUInt32BE(8000, 16); // 8000x5000 = 40MP, right at the cap
    header.writeUInt32BE(5000, 20);
    createFromPath.mockReturnValue(fakeImage({ width: 8000, height: 5000 }));

    expect(() => readImagePreview(write('photo-large.png', header))).not.toThrow();
  });

  it('refuses TIFF rather than inlining bytes no browser can render', () => {
    createFromPath.mockReturnValue(emptyImage);
    expect(() => readImagePreview(write('scan.tiff', 'II*\0'))).toThrow(/no browser renders it/);
  });

  it('refuses to inline an undecodable file that is too big', () => {
    createFromPath.mockReturnValue(emptyImage);
    const p = write('huge.webp', Buffer.alloc(3 * 1024 * 1024));
    expect(() => readImagePreview(p)).toThrow(/too large to inline/);
  });
});

describe('isPreviewableImage', () => {
  it('accepts image extensions case-insensitively and rejects the rest', () => {
    expect(isPreviewableImage('/a/b/Shot.PNG')).toBe(true);
    expect(isPreviewableImage('/a/b/diagram.svg')).toBe(true);
    expect(isPreviewableImage('/a/b/notes.md')).toBe(false);
    expect(isPreviewableImage('/a/b/png')).toBe(false);
  });
});
