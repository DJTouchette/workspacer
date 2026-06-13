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

/** Largest file the editor will open. Bigger files are refused, not truncated. */
const MAX_READ_BYTES = 5 * 1024 * 1024;

export interface ReadFileResult {
  path: string;
  contents: string;
  size: number;
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
