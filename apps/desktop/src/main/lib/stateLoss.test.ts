import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { suspectedStateLoss } from './stateLoss';

/**
 * The table is the same one the Go twin runs
 * (services/hub/internal/statelost/statelost_test.go). Keep them in step: the
 * two halves mint the SAME `<config>/workspacer/remote-token`, so a disagreement
 * about what counts as a first run means one of them silently re-mints while the
 * other refuses.
 */
describe('suspectedStateLoss', () => {
  let root: string;

  beforeEach(() => {
    root = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-stateloss-'));
  });
  afterEach(() => {
    fs.rmSync(root, { recursive: true, force: true });
  });

  const dir = () => path.join(root, 'state');
  // 'dir/'  = an empty directory
  // 'a/b'   = a file inside a directory, so that directory is NOT empty
  // 'name:' = a zero-byte file
  const seed = (entries: string[]) => {
    fs.mkdirSync(dir(), { recursive: true });
    for (const raw of entries) {
      const zeroByte = raw.endsWith(':');
      const e = zeroByte ? raw.slice(0, -1) : raw;
      const p = path.join(dir(), e);
      if (e.endsWith('/')) {
        fs.mkdirSync(p, { recursive: true });
      } else {
        fs.mkdirSync(path.dirname(p), { recursive: true });
        fs.writeFileSync(p, zeroByte ? '' : 'x');
      }
    }
  };

  it('a directory that does not exist is a first run', () => {
    expect(suspectedStateLoss(dir(), 'remote-token')).toBe(false);
  });

  it('an empty directory is a first run', () => {
    seed([]);
    expect(suspectedStateLoss(dir(), 'remote-token')).toBe(false);
  });

  it("the file's own presence does not count as evidence about itself", () => {
    seed(['remote-token']);
    expect(suspectedStateLoss(dir(), 'remote-token')).toBe(false);
  });

  it('any other state means somebody has run here', () => {
    seed(['config.yaml']);
    expect(suspectedStateLoss(dir(), 'remote-token')).toBe(true);
  });

  // The false alarm this used to produce on every genuinely first boot of the
  // Fly node: deploy/fly/node/bootstrap.sh mkdir -p's five directories inside
  // <config>/workspacer before anything runs, and counting any entry read that
  // as "somebody has run here".
  it("an EMPTY subdirectory is a bootstrap's mkdir, not evidence somebody ran here", () => {
    seed(['sessions/']);
    expect(suspectedStateLoss(dir(), 'config.yaml')).toBe(false);
  });

  it('several empty subdirectories are still a first run', () => {
    seed(['plugins/', 'library/', 'layouts/', 'sessions/', 'logs/']);
    expect(suspectedStateLoss(dir(), 'config.yaml')).toBe(false);
  });

  // The other half of the same rule.
  it('a subdirectory with something IN it is real state', () => {
    seed(['sessions/live.json']);
    expect(suspectedStateLoss(dir(), 'config.yaml')).toBe(true);
  });

  it('a file beside empty subdirectories is still real state', () => {
    seed(['plugins/', 'tokens.json']);
    expect(suspectedStateLoss(dir(), 'config.yaml')).toBe(true);
  });

  // An empty FILE is not an empty directory. Something wrote it, so it counts.
  it('a zero-byte neighbour counts, unlike an empty directory', () => {
    seed(['tokens.json:']);
    expect(suspectedStateLoss(dir(), 'config.yaml')).toBe(true);
  });

  it('a truncated file beside real state is still loss', () => {
    seed(['remote-token', 'tokens.json']);
    expect(suspectedStateLoss(dir(), 'remote-token')).toBe(true);
  });
});
