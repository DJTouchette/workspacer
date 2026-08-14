/**
 * Project icons, downloaded once and kept on disk.
 *
 * A project's `favicon` used to be hot-linked: the renderer put the URL straight
 * in an `<img src>`. That is worse than it looks — the mark vanishes offline and
 * whenever the URL rots, the request repeats on every render, and pointing it at
 * a third-party favicon service (the obvious way to get one) tells that service
 * every project domain you work on. So the URL is now fetched ONCE here, written
 * into `<configDir>/project-icons/`, and served to the renderer over the
 * `workspacer-icon://` protocol — the same shape `workspacer-font://` already
 * uses to serve local font files.
 *
 * The URL is still kept in config, as provenance and so the icon can be
 * re-fetched; the cached file is what actually renders.
 */
import * as fs from 'fs';
import * as path from 'path';
import * as crypto from 'crypto';
import { getConfigDir } from './configService';

/** Where downloaded icons live. */
export function projectIconsDir(): string {
  return path.join(getConfigDir(), 'project-icons');
}

/** A downloaded icon is a small square image. These bounds are generous for
 *  that and small enough that a hostile or mistaken URL cannot fill the disk. */
const MAX_BYTES = 2 * 1024 * 1024;
const TIMEOUT_MS = 8000;

/**
 * Image types worth accepting, and the extension each is stored with.
 *
 * The extension comes from the RESPONSE's content type, never from the URL:
 * letting a caller-supplied path decide the filename is how you end up writing
 * `evil.html` (or worse) into a directory the app later serves.
 */
const EXT_BY_MIME: Record<string, string> = {
  'image/png': 'png',
  'image/jpeg': 'jpg',
  'image/gif': 'gif',
  'image/webp': 'webp',
  'image/svg+xml': 'svg',
  'image/x-icon': 'ico',
  'image/vnd.microsoft.icon': 'ico',
  'image/avif': 'avif',
};

export const MIME_BY_EXT: Record<string, string> = {
  png: 'image/png',
  jpg: 'image/jpeg',
  gif: 'image/gif',
  webp: 'image/webp',
  svg: 'image/svg+xml',
  ico: 'image/x-icon',
  avif: 'image/avif',
};

export interface DownloadedIcon {
  /** The stored filename, e.g. `a1b2c3….png`. Config keeps this. */
  file: string;
}

/**
 * Fetch `url` and store it as a project icon. Returns the stored filename.
 *
 * Throws with a human-readable reason rather than returning null: this runs
 * from a paste in the settings UI, where "nothing happened" is the worst
 * possible outcome and the user needs to know whether the URL was wrong, the
 * host was down, or the thing on the other end simply was not an image.
 */
export async function downloadProjectIcon(url: string): Promise<DownloadedIcon> {
  const trimmed = String(url || '').trim();
  let parsed: URL;
  try {
    parsed = new URL(trimmed);
  } catch {
    throw new Error('That is not a URL');
  }
  // http(s) only. `file:` would read anything the app can, and `data:` is not
  // a download at all.
  if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
    throw new Error('Only http and https URLs can be downloaded');
  }

  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), TIMEOUT_MS);
  let res: Response;
  try {
    res = await fetch(parsed.toString(), {
      signal: controller.signal,
      redirect: 'follow',
      headers: { Accept: 'image/*' },
    });
  } catch (err: any) {
    throw new Error(
      err?.name === 'AbortError' ? 'The download timed out' : `Could not reach it: ${err?.message}`,
    );
  } finally {
    clearTimeout(timer);
  }
  if (!res.ok) throw new Error(`The server said ${res.status} ${res.statusText}`);

  const mime = (res.headers.get('content-type') || '').split(';')[0].trim().toLowerCase();
  const ext = EXT_BY_MIME[mime];
  if (!ext) throw new Error(`That is ${mime || 'not an image'}, not an image we can use`);

  const buf = Buffer.from(await res.arrayBuffer());
  if (!buf.length) throw new Error('The file was empty');
  if (buf.length > MAX_BYTES) throw new Error('That image is larger than 2 MB');

  // Content-addressed: the same icon pasted for two projects is stored once,
  // and re-pasting the same URL cannot accumulate copies.
  const file = `${crypto.createHash('sha256').update(buf).digest('hex').slice(0, 32)}.${ext}`;
  const dir = projectIconsDir();
  fs.mkdirSync(dir, { recursive: true });
  const full = path.join(dir, file);
  if (!fs.existsSync(full)) fs.writeFileSync(full, buf);
  return { file };
}

/**
 * Resolve a stored icon filename to a path inside the icons directory, or null.
 *
 * The filename reaches here from config, which is a file a user (or anything
 * that can write it) edits by hand, so it is caller data: it must be one plain
 * segment, and the resolved path must still be inside the directory. Serving
 * `../../.config/workspacer/remote-token` because a config key said so is the
 * whole hazard this closes.
 */
export function resolveProjectIcon(file: string): string | null {
  const name = String(file || '');
  if (!name || name === '.' || name === '..' || /[\\/]/.test(name) || path.isAbsolute(name)) {
    return null;
  }
  const dir = projectIconsDir();
  const full = path.resolve(dir, name);
  if (path.relative(dir, full).startsWith('..')) return null;
  return fs.existsSync(full) ? full : null;
}

/** The content type to serve a stored icon with, from its stored extension. */
export function mimeForIcon(file: string): string {
  return MIME_BY_EXT[path.extname(file).slice(1).toLowerCase()] ?? 'application/octet-stream';
}
