/**
 * Persist the main process's console + daemon output to a file so logs survive
 * the session (previously they lived only in memory and were lost on quit —
 * making bug reports impossible). We tee `process.stdout`/`process.stderr`
 * (which is where both `console.*` and the daemons' piped output go) into a
 * size-capped log file under `<config>/logs/`, and expose the folder so the UI
 * can offer "Open logs".
 */
import * as fs from 'fs';
import * as path from 'path';
import { getConfigDir } from './configService';

const MAX_BYTES = 5 * 1024 * 1024; // rotate at ~5 MB, keep one previous file
let stream: fs.WriteStream | null = null;
/**
 * Bytes in the CURRENT log file. The cap has to be checked on this running
 * total, not once at startup: the startup check alone only bounds the size the
 * log *starts* at, so a long-lived session (which is the normal case — the app
 * stays open for days, teeing every daemon's stdout) grows without any limit.
 * A 12 MB rolled file was the observed result of that.
 */
let written = 0;
/** True between "cap exceeded" and "new stream open" — the swap is async. */
let rotating = false;
/** Idempotency guard for initFileLogging. Can't key on `stream`: rotation
 *  nulls it for a moment, and a second init would double-wrap the tee. */
let teeInstalled = false;

export function logsDir(): string {
  return path.join(getConfigDir(), 'logs');
}

export function logFilePath(): string {
  return path.join(logsDir(), 'workspacer.log');
}

/**
 * Move the current log aside and start a fresh one, keeping exactly one
 * previous file. Writes during the swap go to the console only — a few lines,
 * versus dropping the whole tee.
 *
 * The rename waits for the stream's 'close', not for end()'s callback: Node
 * attaches that callback to 'finish', which fires while the fd is still open.
 * On POSIX renaming an open file works anyway, but on Windows it fails with
 * EPERM/EBUSY — and since reopen() then reattaches to the SAME unrotated file
 * with `written` back at 0, a swallowed failure there means the log grows for
 * ever, 5 MiB per cycle, which is the exact bug this rotation exists to stop.
 * 'close' is the point where the handle is actually gone.
 */
function rotate(): void {
  if (rotating) return;
  rotating = true;
  const old = stream;
  stream = null;
  written = 0;
  const file = logFilePath();
  const reopen = () => {
    try {
      fs.renameSync(file, `${file}.old`);
    } catch (err) {
      // ENOENT is the ordinary case (already rotated, or removed under us).
      // Anything else means the rename genuinely failed and we are about to
      // reopen the un-rotated file, so it must be visible rather than inferred
      // from a log that keeps growing.
      if ((err as NodeJS.ErrnoException)?.code !== 'ENOENT') {
        console.error(`[logging] could not rotate ${file}:`, err);
      }
    }
    try {
      stream = fs.createWriteStream(file, { flags: 'a' });
    } catch {
      /* can't reopen — the tee stays off; console output is unaffected */
    }
    rotating = false;
  };
  if (old) {
    old.once('close', reopen);
    old.end();
  } else reopen();
}

/** Begin teeing stdout/stderr to the log file. Idempotent; call once at startup
 *  BEFORE the daemons spawn so their output is captured too. */
export function initFileLogging(): void {
  if (teeInstalled) return;
  try {
    fs.mkdirSync(logsDir(), { recursive: true });
    const file = logFilePath();
    // Rotate a large existing log to .old so the file doesn't grow unbounded.
    try {
      const size = fs.statSync(file).size;
      if (size > MAX_BYTES) fs.renameSync(file, `${file}.old`);
      // We append, so the existing file's size is already part of the budget —
      // seeding it means a nearly-full log rotates on the next few lines rather
      // than growing to twice the cap first.
      else written = size;
    } catch {
      /* no existing file */
    }
    stream = fs.createWriteStream(file, { flags: 'a' });

    for (const ch of ['stdout', 'stderr'] as const) {
      const orig = process[ch].write.bind(process[ch]);
      (process[ch] as NodeJS.WriteStream).write = ((
        chunk: string | Uint8Array,
        ...rest: unknown[]
      ): boolean => {
        try {
          if (stream) {
            stream.write(typeof chunk === 'string' ? chunk : Buffer.from(chunk));
            written += typeof chunk === 'string' ? Buffer.byteLength(chunk) : chunk.byteLength;
            if (written > MAX_BYTES) rotate();
          }
        } catch {
          /* logging must never throw */
        }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        return (orig as any)(chunk, ...rest);
      }) as NodeJS.WriteStream['write'];
    }
    teeInstalled = true;
    console.log(`[logging] writing logs to ${file}`);
  } catch (err) {
    console.error('[logging] failed to init file logging:', err);
  }
}
