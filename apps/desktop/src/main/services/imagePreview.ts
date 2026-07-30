/**
 * Thumbnail generation for composer image attachments.
 *
 * Attachments are held as host paths, and the renderer can't read those: in dev
 * it is served over http://localhost, so a `file://` <img src> is blocked, and
 * even when it isn't, pointing an <img> at a 12MP photo to draw a 56px tile is
 * wasteful. So main decodes the image and hands back a small `data:` URL.
 *
 * Exposed twice, like the rest of fileService: `file:read-image` IPC for the
 * desktop renderer and the `fs.readImage` hub capability for the web/phone
 * client (whose composer runs the same React code against host files).
 */
import * as fs from 'fs';
import * as path from 'path';
import { nativeImage } from 'electron';
import { readImageDimensions, HEADER_PROBE_BYTES } from './imageHeader';

/** Longest edge of the generated thumbnail. ~2× the largest on-screen tile so
 *  it stays crisp on HiDPI without shipping the full image to the renderer. */
const THUMB_MAX_EDGE = 256;
/** Largest source file we will decode at all. */
const MAX_SOURCE_BYTES = 32 * 1024 * 1024;
/** Cap for the verbatim fallback below — those bytes reach the renderer as-is. */
const MAX_INLINE_BYTES = 2 * 1024 * 1024;
/**
 * Largest image we will hand to the decoder, in pixels. File size does not
 * bound decode cost — a 20000×20000 PNG of flat colour is ~91 KB on disk and
 * 1.6 GB decoded, and nativeImage decodes at full resolution, synchronously, on
 * the main process before any resize can help. 40 MP clears a 50-megapixel-era
 * camera photo while refusing the pathological cases.
 */
const MAX_SOURCE_PIXELS = 40_000_000;

/** Extensions we'll serve, with the MIME the fallback path embeds them under.
 *  Doubles as the allow-list: anything else is refused, so neither the IPC nor
 *  the hub capability can be used to base64 an arbitrary file. */
const MIME_BY_EXT: Record<string, string> = {
  png: 'image/png',
  jpg: 'image/jpeg',
  jpeg: 'image/jpeg',
  gif: 'image/gif',
  webp: 'image/webp',
  svg: 'image/svg+xml',
  bmp: 'image/bmp',
  ico: 'image/x-icon',
  tiff: 'image/tiff',
  tif: 'image/tiff',
  avif: 'image/avif',
};

export interface ImagePreview {
  path: string;
  /** Downscaled thumbnail as a `data:` URL, ready for an <img src>. */
  dataUrl: string;
  /** Natural pixel size of the source. 0 when the format wasn't decoded
   *  (inline fallback) — SVG in particular has no intrinsic pixel size. */
  width: number;
  height: number;
  /** Size of the source file in bytes. */
  size: number;
}

/** Of the allowed extensions, the ones an <img> can display if we hand over the
 *  original bytes. Excludes TIFF, which no browser renders. */
const BROWSER_RENDERABLE = new Set([
  'png',
  'jpg',
  'jpeg',
  'gif',
  'webp',
  'svg',
  'bmp',
  'ico',
  'avif',
]);

/** First bytes of a file, for header sniffing. Returns null if unreadable — the
 *  caller then proceeds without a dimension check rather than failing. */
function readHeader(filePath: string): Buffer | null {
  let fd: number | null = null;
  try {
    fd = fs.openSync(filePath, 'r');
    const buf = Buffer.alloc(HEADER_PROBE_BYTES);
    const read = fs.readSync(fd, buf, 0, HEADER_PROBE_BYTES, 0);
    return buf.subarray(0, read);
  } catch {
    return null;
  } finally {
    if (fd !== null) fs.closeSync(fd);
  }
}

export function isPreviewableImage(filePath: string): boolean {
  const ext = path.extname(filePath).slice(1).toLowerCase();
  return ext in MIME_BY_EXT;
}

/**
 * Decode `filePath` and return a thumbnail data URL. Throws on anything that
 * isn't a readable image inside the size caps; callers treat a rejection as
 * "no preview" and fall back to a plain chip.
 */
export function readImagePreview(filePath: string): ImagePreview {
  const ext = path.extname(filePath).slice(1).toLowerCase();
  const mime = MIME_BY_EXT[ext];
  if (!mime) throw new Error(`not a previewable image: ${filePath}`);

  const stat = fs.statSync(filePath); // throws ENOENT etc. → surfaced to caller
  if (!stat.isFile()) throw new Error(`not a regular file: ${filePath}`);
  if (stat.size > MAX_SOURCE_BYTES) {
    throw new Error(`image is ${stat.size} bytes (max ${MAX_SOURCE_BYTES})`);
  }

  // Refuse decode bombs before the decoder ever sees them. Formats whose header
  // we don't parse return null — those fall through, bounded only by
  // MAX_SOURCE_BYTES, which is the pre-existing behaviour.
  const probe = readHeader(filePath);
  const dims = probe && readImageDimensions(probe);
  if (dims && dims.width * dims.height > MAX_SOURCE_PIXELS) {
    throw new Error(
      `image is ${dims.width}×${dims.height} (max ${MAX_SOURCE_PIXELS} pixels to decode)`,
    );
  }

  // Chromium's decoder handles PNG/JPEG everywhere and a few more formats
  // platform-dependently; anything it can't read comes back as an empty image
  // rather than an error, which is the signal to fall back.
  const img = nativeImage.createFromPath(filePath);
  if (!img.isEmpty()) {
    const { width, height } = img.getSize();
    const longest = Math.max(width, height);
    const scale = longest > THUMB_MAX_EDGE ? THUMB_MAX_EDGE / longest : 1;
    const thumb =
      scale < 1
        ? img.resize({
            width: Math.max(1, Math.round(width * scale)),
            height: Math.max(1, Math.round(height * scale)),
            quality: 'good',
          })
        : img;
    // JPEG for photos (a resized photo is several times smaller as JPEG),
    // PNG — via toDataURL — for everything else so transparency survives.
    const dataUrl =
      mime === 'image/jpeg'
        ? `data:image/jpeg;base64,${thumb.toJPEG(78).toString('base64')}`
        : thumb.toDataURL();
    return { path: filePath, dataUrl, width, height, size: stat.size };
  }

  // Undecodable here but very possibly decodable by the renderer's own <img>
  // (SVG, and WebP/AVIF on platforms where nativeImage declines): embed the
  // original bytes, but only while they're small enough to be worth it.
  //
  // Only for formats a browser can actually render. TIFF is the reason: no
  // browser displays it, so inlining it produced a broken-image tile where the
  // plain chip (what a rejection gives you) is the honest result.
  if (!BROWSER_RENDERABLE.has(ext)) {
    throw new Error(`cannot decode ${ext}, and no browser renders it either`);
  }
  if (stat.size > MAX_INLINE_BYTES) {
    throw new Error(`cannot decode ${ext} and it is too large to inline (${stat.size} bytes)`);
  }
  const data = fs.readFileSync(filePath).toString('base64');
  return {
    path: filePath,
    dataUrl: `data:${mime};base64,${data}`,
    width: 0,
    height: 0,
    size: stat.size,
  };
}
