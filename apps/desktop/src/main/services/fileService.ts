/**
 * Generic text-file read/write for the editor pane.
 *
 * This is the app's own file backend (the renderer is the editor; main is its
 * trusted fs layer) — deliberately NOT in claudemon (session daemon) or the hub
 * (pure router). It's exposed two ways so both clients can edit the same files
 * on the host: `file:read`/`file:write` IPC for the desktop renderer, and
 * `fs.read`/`fs.write` hub capabilities for the web/phone client.
 *
 * Reads refuse oversized, non-regular, binary, or non-UTF-8 files so the editor
 * never loads something it would silently corrupt when it saves the buffer back.
 */
import * as fs from 'fs';
import * as path from 'path';
import { execFileSync } from 'child_process';

/** Largest file the editor will open. Bigger files are refused, not truncated. */
const MAX_READ_BYTES = 5 * 1024 * 1024;

export interface ReadFileResult {
  path: string;
  contents: string;
  size: number;
}

export interface DirEntry {
  name: string;
  path: string;
  isDir: boolean;
}
export interface ListDirResult {
  path: string;
  entries: DirEntry[];
}

/**
 * List a single directory level for the editor's file tree. Always hides `.git`;
 * within a git repo it also hides anything matched by `.gitignore` (using git's
 * own logic via `git check-ignore`, so nested ignore files are honoured). One
 * git invocation per directory expand — fine for interactive browsing.
 */
export function listDir(dirPath: string): ListDirResult {
  const resolved = path.resolve(dirPath);
  const dirents = fs
    .readdirSync(resolved, { withFileTypes: true })
    .filter((e) => e.name !== '.git');

  // Ask git which of these names are ignored (batched over stdin). We use `-z`
  // so paths are NUL-delimited on both stdin and stdout: a filename may legally
  // contain a newline, and a linefeed-delimited protocol would split it into
  // two bogus paths so the echoed match never equals the readdir name.
  let ignored = new Set<string>();
  if (dirents.length) {
    const names = dirents.map((e) => e.name).join('\0');
    try {
      // core.quotePath=false keeps non-ASCII names unquoted so they match the
      // decoded names fs.readdir returns (otherwise git emits e.g. "\303\251.log"
      // and the ignore filter silently misses unicode-named files).
      const out = execFileSync(
        'git',
        ['-c', 'core.quotePath=false', 'check-ignore', '-z', '--stdin'],
        {
          cwd: resolved,
          input: names,
          encoding: 'utf8',
        },
      );
      ignored = new Set(out.split('\0').filter(Boolean));
    } catch (err) {
      // exit 1 = nothing ignored (stdout still holds any matches); 128 = not a
      // git repo / git missing → no filtering at all.
      const e = err as { status?: number; stdout?: string };
      if (e.status === 1 && typeof e.stdout === 'string') {
        ignored = new Set(e.stdout.split('\0').filter(Boolean));
      }
    }
  }

  const entries: DirEntry[] = dirents
    .filter((e) => !ignored.has(e.name))
    .map((e) => {
      const full = path.join(resolved, e.name);
      let isDir = e.isDirectory();
      if (!isDir && e.isSymbolicLink()) {
        try {
          isDir = fs.statSync(full).isDirectory();
        } catch {
          /* dangling link */
        }
      }
      return { name: e.name, path: full, isDir };
    })
    // Directories first, then alphabetical (locale-aware).
    .sort((a, b) => (a.isDir === b.isDir ? a.name.localeCompare(b.name) : a.isDir ? -1 : 1));

  return { path: resolved, entries };
}

export function readTextFile(filePath: string): ReadFileResult {
  const stat = fs.statSync(filePath); // throws ENOENT etc. → surfaced to caller
  if (!stat.isFile()) throw new Error(`not a regular file: ${filePath}`);
  if (stat.size > MAX_READ_BYTES) {
    throw new Error(`file is ${stat.size} bytes (max ${MAX_READ_BYTES})`);
  }
  const buf = fs.readFileSync(filePath);
  if (buf.includes(0)) throw new Error('file appears to be binary');
  const contents = buf.toString('utf8');
  // toString('utf8') is lossy (invalid bytes → U+FFFD); refuse rather than risk
  // clobbering the file on save with replacement characters.
  if (!Buffer.from(contents, 'utf8').equals(buf)) {
    throw new Error('file is not valid UTF-8');
  }
  return { path: filePath, contents, size: stat.size };
}

export function writeTextFile(filePath: string, contents: string): { ok: true } {
  fs.writeFileSync(filePath, contents, 'utf8');
  return { ok: true };
}
