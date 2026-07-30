/**
 * Header-only dimension parsing. This exists to refuse decode bombs before
 * nativeImage sees them — file size does not bound decode cost (a 20000×20000
 * flat-colour PNG is ~91 KB on disk and gigabytes decoded), so the numbers
 * these parsers return are what stands between a large image and a frozen main
 * process. A parser that silently returns null degrades to the old behaviour,
 * so the cases below check both the values and the honest-null paths.
 */
import { describe, it, expect } from 'vitest';
import { readImageDimensions } from './imageHeader';

function png(width: number, height: number): Buffer {
  const buf = Buffer.alloc(24);
  buf.writeUInt32BE(0x89504e47, 0);
  buf.writeUInt32BE(0x0d0a1a0a, 4);
  buf.write('IHDR', 12, 'ascii');
  buf.writeUInt32BE(width, 16);
  buf.writeUInt32BE(height, 20);
  return buf;
}

function gif(width: number, height: number): Buffer {
  const buf = Buffer.alloc(10);
  buf.write('GIF89a', 0, 'ascii');
  buf.writeUInt16LE(width, 6);
  buf.writeUInt16LE(height, 8);
  return buf;
}

function bmp(width: number, height: number): Buffer {
  const buf = Buffer.alloc(26);
  buf.write('BM', 0, 'ascii');
  buf.writeInt32LE(width, 18);
  buf.writeInt32LE(height, 22);
  return buf;
}

/** JPEG with an EXIF-ish APP1 segment before the frame header, so the walker
 *  has to skip a segment rather than find SOF0 immediately. */
function jpeg(width: number, height: number, appLength = 40): Buffer {
  const parts = [Buffer.from([0xff, 0xd8])];
  const app = Buffer.alloc(2 + appLength);
  app.writeUInt16BE(0xffe1, 0);
  app.writeUInt16BE(appLength, 2);
  parts.push(app);
  const sof = Buffer.alloc(11);
  sof.writeUInt16BE(0xffc0, 0);
  sof.writeUInt16BE(9, 2); // segment length
  sof.writeUInt8(8, 4); // precision
  sof.writeUInt16BE(height, 5);
  sof.writeUInt16BE(width, 7);
  parts.push(sof);
  return Buffer.concat(parts);
}

function webpLossy(width: number, height: number): Buffer {
  const buf = Buffer.alloc(30);
  buf.write('RIFF', 0, 'ascii');
  buf.write('WEBP', 8, 'ascii');
  buf.write('VP8 ', 12, 'ascii');
  buf.writeUInt16LE(width, 26);
  buf.writeUInt16LE(height, 28);
  return buf;
}

describe('readImageDimensions', () => {
  it('reads PNG', () => {
    expect(readImageDimensions(png(1920, 1080))).toEqual({ width: 1920, height: 1080 });
  });

  it('reads the decode bomb this guard exists for', () => {
    expect(readImageDimensions(png(20000, 20000))).toEqual({ width: 20000, height: 20000 });
  });

  it('reads GIF (little-endian, unlike PNG)', () => {
    expect(readImageDimensions(gif(640, 480))).toEqual({ width: 640, height: 480 });
  });

  it('reads BMP, normalising a bottom-up negative height', () => {
    expect(readImageDimensions(bmp(800, -600))).toEqual({ width: 800, height: 600 });
  });

  it('reads JPEG past a preceding APP segment', () => {
    expect(readImageDimensions(jpeg(4032, 3024))).toEqual({ width: 4032, height: 3024 });
  });

  it('reads lossy WebP', () => {
    expect(readImageDimensions(webpLossy(1200, 630))).toEqual({ width: 1200, height: 630 });
  });

  it('returns null for a format it does not parse, rather than a wrong number', () => {
    expect(
      readImageDimensions(Buffer.from('<svg xmlns="http://www.w3.org/2000/svg"/>')),
    ).toBeNull();
    expect(readImageDimensions(Buffer.from('II*\0 tiff-ish'))).toBeNull();
  });

  it('returns null on truncated or empty input instead of throwing', () => {
    expect(readImageDimensions(Buffer.alloc(0))).toBeNull();
    expect(readImageDimensions(png(10, 10).subarray(0, 18))).toBeNull();
    expect(readImageDimensions(Buffer.from([0xff, 0xd8, 0xff]))).toBeNull();
  });

  it('does not loop forever on a JPEG with a malformed segment length', () => {
    const bad = Buffer.concat([
      Buffer.from([0xff, 0xd8]),
      Buffer.from([0xff, 0xe1, 0x00, 0x00]), // length 0 — smaller than its own field
      jpeg(100, 100).subarray(2),
    ]);
    expect(readImageDimensions(bad)).toBeNull();
  });
});
