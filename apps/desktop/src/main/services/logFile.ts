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

export function logsDir(): string {
  return path.join(getConfigDir(), 'logs');
}

export function logFilePath(): string {
  return path.join(logsDir(), 'workspacer.log');
}

/** Begin teeing stdout/stderr to the log file. Idempotent; call once at startup
 *  BEFORE the daemons spawn so their output is captured too. */
export function initFileLogging(): void {
  if (stream) return;
  try {
    fs.mkdirSync(logsDir(), { recursive: true });
    const file = logFilePath();
    // Rotate a large existing log to .old so the file doesn't grow unbounded.
    try {
      if (fs.statSync(file).size > MAX_BYTES) fs.renameSync(file, `${file}.old`);
    } catch { /* no existing file */ }
    stream = fs.createWriteStream(file, { flags: 'a' });

    for (const ch of ['stdout', 'stderr'] as const) {
      const orig = process[ch].write.bind(process[ch]);
      (process[ch] as NodeJS.WriteStream).write = ((chunk: string | Uint8Array, ...rest: unknown[]): boolean => {
        try { stream?.write(typeof chunk === 'string' ? chunk : Buffer.from(chunk)); } catch { /* logging must never throw */ }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        return (orig as any)(chunk, ...rest);
      }) as NodeJS.WriteStream['write'];
    }
    console.log(`[logging] writing logs to ${file}`);
  } catch (err) {
    console.error('[logging] failed to init file logging:', err);
  }
}
