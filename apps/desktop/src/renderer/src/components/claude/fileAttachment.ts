// ── File Attachment Helpers ──

export const IMAGE_EXTS = new Set([
  'png',
  'jpg',
  'jpeg',
  'gif',
  'webp',
  'svg',
  'bmp',
  'ico',
  'tiff',
]);
export const PDF_EXTS = new Set(['pdf']);

export interface AttachedFile {
  path: string;
  label: string; // "Image" | "PDF" | "File"
  name: string; // basename
}

export function classifyFile(filePath: string): AttachedFile {
  const name = filePath.split(/[/\\]/).pop() ?? filePath;
  const ext = name.split('.').pop()?.toLowerCase() ?? '';
  const label = IMAGE_EXTS.has(ext) ? 'Image' : PDF_EXTS.has(ext) ? 'PDF' : 'File';
  return { path: filePath, label, name };
}

/**
 * Add attachments, skipping paths already attached. Attaching the same file
 * twice is never meaningful — it would list the path twice in the prompt prefix
 * and give two chips the same React key — and it's easy to do by accident
 * (drop, then drop again because the first drop looked like it missed).
 */
export function mergeAttachments(
  existing: AttachedFile[],
  incoming: AttachedFile[],
): AttachedFile[] {
  const seen = new Set(existing.map((f) => f.path));
  const added = incoming.filter((f) => !seen.has(f.path) && (seen.add(f.path), true));
  return added.length > 0 ? [...existing, ...added] : existing;
}

export function buildPromptPrefix(files: AttachedFile[]): string {
  return files.map((f) => `[${f.label}: ${f.path}]`).join(' ') + ' ';
}

/**
 * A file:// URL as a filesystem path. `pathname` alone is wrong on Windows: a
 * drive path arrives as `/C:/dir/x.png` (leading slash), and a network share as
 * a hostname the path drops entirely.
 */
function fileUriToPath(url: URL): string {
  const decoded = decodeURIComponent(url.pathname);
  if (url.hostname) return `\\\\${url.hostname}${decoded.replace(/\//g, '\\')}`;
  return /^\/[A-Za-z]:/.test(decoded) ? decoded.slice(1) : decoded;
}

/**
 * Extract host file paths from a drop or paste.
 *
 * Attachments are paths (the agent reads the file itself), so a dropped File is
 * only useful once resolved back to where it lives on disk. Electron 32 removed
 * the `File.path` augmentation that used to make that trivial — but kept its
 * type declaration in electron.d.ts, so reading `f.path` still compiles and just
 * yields undefined at runtime. The replacement, `webUtils.getPathForFile`, is
 * renderer-side only, so it's bridged through preload as `getPathForFile`.
 *
 * Falls back to `text/uri-list` (some drag sources supply file:// URIs without
 * populating `files`) and then to the legacy property, so this keeps working on
 * older Electron and doesn't hard-depend on the bridge existing.
 */
export function extractFilePaths(dataTransfer: DataTransfer): string[] {
  const resolve = window.electronAPI?.getPathForFile;
  const paths: string[] = [];

  for (const f of Array.from(dataTransfer.files ?? [])) {
    // '' means a File not backed by disk — a pasted screenshot, say. There's no
    // path to attach, so skip it rather than push an empty entry.
    const viaBridge = resolve ? resolve(f) : '';
    const legacy = (f as File & { path?: string }).path;
    const p = viaBridge || legacy;
    if (p) paths.push(p);
  }
  if (paths.length > 0) return paths;

  // Nothing resolved from `files` — try the URI list. Only file:// entries: a
  // drag from a web page carries an http(s) URI, which is not an attachment.
  const uriList = dataTransfer.getData('text/uri-list');
  if (!uriList) return paths;
  for (const line of uriList.split(/\r?\n/)) {
    const uri = line.trim();
    if (!uri || uri.startsWith('#')) continue; // '#' lines are uri-list comments
    if (!uri.startsWith('file://')) continue;
    try {
      paths.push(fileUriToPath(new URL(uri)));
    } catch {
      /* malformed URI — skip */
    }
  }
  return paths;
}
