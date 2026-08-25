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
  const seed = (entries: string[]) => {
    fs.mkdirSync(dir(), { recursive: true });
    for (const e of entries) {
      if (e.endsWith('/')) fs.mkdirSync(path.join(dir(), e), { recursive: true });
      else fs.writeFileSync(path.join(dir(), e), 'x');
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

  it('a subdirectory counts too', () => {
    seed(['sessions/']);
    expect(suspectedStateLoss(dir(), 'config.yaml')).toBe(true);
  });

  it('a truncated file beside real state is still loss', () => {
    seed(['remote-token', 'tokens.json']);
    expect(suspectedStateLoss(dir(), 'remote-token')).toBe(true);
  });
});
