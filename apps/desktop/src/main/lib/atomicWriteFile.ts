/**
 * Crash-safe file writes: write to a unique temp file in the SAME directory,
 * then `fs.renameSync` it over the target. A rename within one filesystem is
 * atomic, so a reader (or a crash / power-loss mid-write) sees either the old,
 * complete file or the new, complete file — never a half-written one. The plain
 * `fs.writeFileSync(finalPath, …)` it replaces truncates the target first, so an
 * interruption there leaves the config/layout/session file corrupt.
 *
 * The temp file lives beside the target (never in $TMPDIR) so the rename stays
 * on the same filesystem; on any failure it's cleaned up and the original file
 * is left untouched. Extracted from remoteTokens.ts's writeTokens(), which was
 * the one place that already did this correctly, so there's a single impl.
 */
import * as fs from 'fs';
import * as path from 'path';

export interface AtomicWriteOptions {
  /** File mode for the created file (e.g. 0o600 for secrets like tokens.json). */
  mode?: number;
  /** Text encoding for string data. Default 'utf-8'. */
  encoding?: BufferEncoding;
}

// Monotonic counter so two writes in the same millisecond (same pid) still get
// distinct temp names — Date.now() alone can collide under a tight loop.
let seq = 0;

/**
 * Atomically write `data` to `filePath`. Creates the parent directory if
 * needed. Preserves `opts.mode` (chmod'd after write for filesystems that
 * ignore the open-time mode) and `opts.encoding` (default utf-8).
 */
export function atomicWriteFileSync(
  filePath: string,
  data: string | NodeJS.ArrayBufferView,
  opts: AtomicWriteOptions = {},
): void {
  const dir = path.dirname(filePath);
  fs.mkdirSync(dir, { recursive: true });

  // Hidden dotfile with a `.tmp-` infix so it never collides with the target and
  // is never mistaken for a real file (e.g. layoutService/sessionService list
  // only `*.yaml`, so a leftover temp is ignored, not loaded).
  const tmp = path.join(
    dir,
    `.${path.basename(filePath)}.tmp-${process.pid}-${Date.now()}-${seq++}`,
  );

  const writeOpts: { mode?: number; encoding?: BufferEncoding } = {
    encoding: opts.encoding ?? 'utf-8',
  };
  if (opts.mode !== undefined) writeOpts.mode = opts.mode;

  try {
    fs.writeFileSync(tmp, data, writeOpts);
    if (opts.mode !== undefined) {
      try {
        fs.chmodSync(tmp, opts.mode);
      } catch {
        /* best effort on filesystems that ignore POSIX modes */
      }
    }
    fs.renameSync(tmp, filePath);
  } catch (err) {
    try {
      fs.rmSync(tmp, { force: true });
    } catch {
      /* best effort cleanup — the original target is untouched regardless */
    }
    throw err;
  }
}
