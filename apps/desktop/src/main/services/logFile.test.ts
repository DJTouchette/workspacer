/**
 * The 5 MB cap has to hold for the LIFETIME of a session, not just at startup.
 * The app runs for days with every daemon's stdout teed into this file, so a
 * once-at-init size check bounds only the size the log starts at — which is how
 * a 12 MB rolled log happened. These tests drive real writes through the
 * installed tee and assert the running total triggers rotation.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

let dir = '';
vi.mock('./configService', () => ({ getConfigDir: () => configDir }));
let configDir = '';

/**
 * Seams into the two fs calls rotation is built out of. `vi.spyOn(fs, …)` can't
 * reach them — the ESM namespace isn't configurable — and the ordering these
 * tests exist to pin (rename strictly after the fd is closed) is only
 * observable from inside the calls themselves. Both default to the real thing,
 * so every other test in this file still exercises real files.
 */
const fsHooks = vi.hoisted(() => ({
  rename: null as ((from: string, to: string) => void) | null,
  onCreate: null as ((s: unknown) => void) | null,
}));
vi.mock('fs', async (importOriginal) => {
  const actual = await importOriginal<typeof import('fs')>();
  const mod = {
    ...actual,
    renameSync: (...args: Parameters<typeof actual.renameSync>) =>
      fsHooks.rename
        ? fsHooks.rename(args[0] as string, args[1] as string)
        : actual.renameSync(...args),
    createWriteStream: (...args: Parameters<typeof actual.createWriteStream>) => {
      const s = actual.createWriteStream(...args);
      fsHooks.onCreate?.(s);
      return s;
    },
  };
  return { ...mod, default: mod };
});

const origStdout = process.stdout.write;
const origStderr = process.stderr.write;

beforeEach(() => {
  dir = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-log-'));
  configDir = dir;
  fsHooks.rename = null;
  fsHooks.onCreate = null;
  vi.resetModules();
  // Swallow the real console output BEFORE the tee is installed: initFileLogging
  // binds whatever write() is current as its passthrough, so this keeps the
  // megabytes we push through it out of the test runner's output.
  process.stdout.write = (() => true) as NodeJS.WriteStream['write'];
  process.stderr.write = (() => true) as NodeJS.WriteStream['write'];
});

afterEach(() => {
  vi.restoreAllMocks();
  process.stdout.write = origStdout;
  process.stderr.write = origStderr;
  fs.rmSync(dir, { recursive: true, force: true });
});

/** Wait for the async stream close + rename that rotation does. */
async function waitFor(pred: () => boolean, ms = 5000): Promise<void> {
  const deadline = Date.now() + ms;
  while (Date.now() < deadline) {
    if (pred()) return;
    await new Promise((r) => setTimeout(r, 20));
  }
  throw new Error('timed out waiting for condition');
}

const MB = 1024 * 1024;

describe('initFileLogging rotation', () => {
  it('rotates on the running total, not just the size at startup', async () => {
    const mod = await import('./logFile');
    mod.initFileLogging();
    const file = mod.logFilePath();

    // Six megabytes written AFTER the tee is installed — the startup check saw
    // an empty (nonexistent) file, so only a running total can catch this.
    const chunk = `${'x'.repeat(MB - 1)}\n`;
    for (let i = 0; i < 6; i++) process.stdout.write(chunk);

    await waitFor(() => fs.existsSync(`${file}.old`));
    expect(fs.statSync(`${file}.old`).size).toBeGreaterThan(5 * MB);
    // The live log restarted, so it is bounded again.
    await waitFor(() => fs.existsSync(file));
    expect(fs.statSync(file).size).toBeLessThan(MB);
  });

  it('keeps teeing after a rotation', async () => {
    const mod = await import('./logFile');
    mod.initFileLogging();
    const file = mod.logFilePath();

    const chunk = `${'x'.repeat(MB - 1)}\n`;
    for (let i = 0; i < 6; i++) process.stdout.write(chunk);
    await waitFor(() => fs.existsSync(`${file}.old`) && fs.existsSync(file));

    process.stderr.write('after-rotation marker\n');
    await waitFor(() => fs.readFileSync(file, 'utf-8').includes('after-rotation marker'));
  });

  it('counts a pre-existing log toward the budget instead of restarting it', async () => {
    const mod = await import('./logFile');
    fs.mkdirSync(mod.logsDir(), { recursive: true });
    const file = mod.logFilePath();
    // Just under the cap: appending to it must rotate almost immediately rather
    // than letting the file reach twice the cap.
    fs.writeFileSync(file, 'y'.repeat(5 * MB - 1024));

    mod.initFileLogging();
    expect(fs.existsSync(`${file}.old`)).toBe(false); // not rotated at startup

    process.stdout.write(`${'x'.repeat(4096)}\n`);
    await waitFor(() => fs.existsSync(`${file}.old`));
    expect(fs.statSync(file).size).toBeLessThan(MB);
  });

  it('closes the old stream before renaming it', async () => {
    // The rename must not race the fd. POSIX lets you rename an open file, so
    // the ordering bug is invisible here — on Windows it raises EPERM/EBUSY,
    // rotation silently fails, and the log grows for ever. Assert the ordering
    // itself rather than a platform's tolerance of getting it wrong.
    const opened: fs.WriteStream[] = [];
    fsHooks.onCreate = (s) => opened.push(s as fs.WriteStream);

    let closedAtRename: boolean | undefined;
    fsHooks.rename = (from, to) => {
      closedAtRename = opened[0]?.closed;
      fsHooks.rename = null; // step aside so the delegation below isn't re-entrant
      fs.renameSync(from, to);
    };

    const mod = await import('./logFile');
    mod.initFileLogging();
    const file = mod.logFilePath();

    const chunk = `${'x'.repeat(MB - 1)}\n`;
    for (let i = 0; i < 6; i++) process.stdout.write(chunk);
    await waitFor(() => fs.existsSync(`${file}.old`));

    expect(closedAtRename).toBe(true);
  });

  it('reports a rename that genuinely failed instead of swallowing it', async () => {
    // A swallowed EBUSY leaves the tee writing to the un-rotated file with the
    // byte counter reset, i.e. unbounded growth that looks like nothing is
    // wrong. ENOENT stays quiet — that one is the ordinary "already gone" case.
    const errors: unknown[][] = [];
    vi.spyOn(console, 'error').mockImplementation((...a: unknown[]) => {
      errors.push(a);
    });
    fsHooks.rename = () => {
      const err = new Error('EBUSY: resource busy or locked') as NodeJS.ErrnoException;
      err.code = 'EBUSY';
      throw err;
    };

    const mod = await import('./logFile');
    mod.initFileLogging();

    const chunk = `${'x'.repeat(MB - 1)}\n`;
    for (let i = 0; i < 6; i++) process.stdout.write(chunk);
    await waitFor(() => errors.some((a) => String(a[0]).includes('could not rotate')));
  });

  it('rotates an oversized log at startup, as before', async () => {
    const mod = await import('./logFile');
    fs.mkdirSync(mod.logsDir(), { recursive: true });
    const file = mod.logFilePath();
    fs.writeFileSync(file, 'y'.repeat(6 * MB));

    mod.initFileLogging();
    expect(fs.existsSync(`${file}.old`)).toBe(true);
    // createWriteStream opens asynchronously, so the fresh log appears a tick later.
    await waitFor(() => fs.existsSync(file));
    expect(fs.statSync(file).size).toBeLessThan(MB);
  });
});
