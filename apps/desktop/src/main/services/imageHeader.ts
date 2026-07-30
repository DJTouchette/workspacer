/**
 * Pixel dimensions read from an image file's header bytes.
 *
 * Needed because file size is a terrible proxy for decode cost: a 20000×20000
 * PNG of flat colour is ~91 KB on disk and 1.6 GB decoded. `nativeImage`
 * decodes at full resolution *before* anything can downscale it, and it does so
 * synchronously on the main process — measured at 1.4s of frozen UI and a 1.8 GB
 * RSS spike for one such file. Reading a few header bytes first lets us refuse
 * the file instead of wearing that.
 *
 * Returns null for formats not parsed here; the caller decides what to do with
 * an unknown size (see imagePreview.ts).
 */

export interface ImageDimensions {
  width: number;
  height: number;
}

/** Enough for every header below, including a JPEG with a fat EXIF block. */
export const HEADER_PROBE_BYTES = 65536;

export function readImageDimensions(buf: Buffer): ImageDimensions | null {
  return png(buf) ?? gif(buf) ?? bmp(buf) ?? webp(buf) ?? jpeg(buf);
}

/** \x89PNG\r\n\x1a\n, then an IHDR chunk whose first 8 bytes are w/h (BE). */
function png(buf: Buffer): ImageDimensions | null {
  if (buf.length < 24) return null;
  if (buf.readUInt32BE(0) !== 0x89504e47 || buf.readUInt32BE(4) !== 0x0d0a1a0a) return null;
  if (buf.toString('ascii', 12, 16) !== 'IHDR') return null;
  return { width: buf.readUInt32BE(16), height: buf.readUInt32BE(20) };
}

/** GIF87a/GIF89a: logical screen size at offset 6, little-endian u16. */
function gif(buf: Buffer): ImageDimensions | null {
  if (buf.length < 10) return null;
  const magic = buf.toString('ascii', 0, 6);
  if (magic !== 'GIF87a' && magic !== 'GIF89a') return null;
  return { width: buf.readUInt16LE(6), height: buf.readUInt16LE(8) };
}

/** BM + DIB header; width/height are signed (height < 0 = top-down rows). */
function bmp(buf: Buffer): ImageDimensions | null {
  if (buf.length < 26 || buf.toString('ascii', 0, 2) !== 'BM') return null;
  return { width: Math.abs(buf.readInt32LE(18)), height: Math.abs(buf.readInt32LE(22)) };
}

/** RIFF….WEBP, then one of three chunk layouts. */
function webp(buf: Buffer): ImageDimensions | null {
  if (buf.length < 30) return null;
  if (buf.toString('ascii', 0, 4) !== 'RIFF' || buf.toString('ascii', 8, 12) !== 'WEBP') {
    return null;
  }
  const chunk = buf.toString('ascii', 12, 16);
  // Lossy: 14-bit dimensions after the 3-byte start code + 0x9d012a signature.
  if (chunk === 'VP8 ') {
    return { width: buf.readUInt16LE(26) & 0x3fff, height: buf.readUInt16LE(28) & 0x3fff };
  }
  // Lossless: 14-bit each, packed across 4 bytes after the 0x2f signature.
  if (chunk === 'VP8L') {
    const bits = buf.readUInt32LE(21);
    return { width: (bits & 0x3fff) + 1, height: ((bits >> 14) & 0x3fff) + 1 };
  }
  // Extended: 24-bit canvas size, stored minus one.
  if (chunk === 'VP8X') {
    const at = (o: number) => buf[o] | (buf[o + 1] << 8) | (buf[o + 2] << 16);
    return { width: at(24) + 1, height: at(27) + 1 };
  }
  return null;
}

/** JPEG: walk the segment chain to the frame header (SOFn) and read its size. */
function jpeg(buf: Buffer): ImageDimensions | null {
  if (buf.length < 4 || buf.readUInt16BE(0) !== 0xffd8) return null;
  let offset = 2;
  while (offset + 9 < buf.length) {
    if (buf[offset] !== 0xff) {
      offset++; // resync past padding/fill bytes rather than giving up
      continue;
    }
    const marker = buf[offset + 1];
    // Standalone markers carry no length payload.
    if (marker === 0xd8 || marker === 0x01 || (marker >= 0xd0 && marker <= 0xd7)) {
      offset += 2;
      continue;
    }
    // SOF0-3, 5-7, 9-11, 13-15 are frame headers (DHT/JPG/DAC are not).
    const isFrame =
      marker >= 0xc0 && marker <= 0xcf && marker !== 0xc4 && marker !== 0xc8 && marker !== 0xcc;
    if (isFrame) {
      return { height: buf.readUInt16BE(offset + 5), width: buf.readUInt16BE(offset + 7) };
    }
    const length = buf.readUInt16BE(offset + 2);
    if (length < 2) return null; // malformed: a length must cover its own field
    offset += 2 + length;
  }
  return null;
}
