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

/** Longest edge of the generated thumbnail. ~2× the largest on-screen tile so
 *  it stays crisp on HiDPI without shipping the full image to the renderer. */
const THUMB_MAX_EDGE = 256;
/** Largest source file we will decode at all. */
const MAX_SOURCE_BYTES = 32 * 1024 * 1024;
/** Cap for the verbatim fallback below — those bytes reach the renderer as-is. */
const MAX_INLINE_BYTES = 2 * 1024 * 1024;

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
