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

export function buildPromptPrefix(files: AttachedFile[]): string {
  return files.map((f) => `[${f.label}: ${f.path}]`).join(' ') + ' ';
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
      paths.push(decodeURIComponent(new URL(uri).pathname));
    } catch {
      /* malformed URI — skip */
    }
  }
  return paths;
}
