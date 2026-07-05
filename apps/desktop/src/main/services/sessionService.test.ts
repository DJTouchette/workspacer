/**
 * SECURITY.md #7: loadSession / deleteSession take a caller-supplied filename and
 * are reachable from the hub bus (the sessions.load / sessions.delete caps), so a
 * traversal like "../../.ssh/id_rsa" must be rejected before touching the disk.
 */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import * as os from 'os';
import * as path from 'path';
import * as fs from 'fs';

// A per-test temp config dir stands in for ~/.workspacer; sessions live under it.
let configDir: string;
vi.mock('./configService', () => ({
  getConfigDir: () => configDir,
}));
// The client is only imported for getCwd (session enrichment); stub it out.
vi.mock('./claudemonSessionClient', () => ({
  claudemonSessionClient: { getCwd: () => undefined },
}));

const { sessionService } = await import('./sessionService');

let sessionsDir: string;
let secretOutside: string;

beforeEach(() => {
  configDir = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-cfg-')));
  sessionsDir = path.join(configDir, 'sessions');
  fs.mkdirSync(sessionsDir, { recursive: true });
  fs.writeFileSync(path.join(sessionsDir, 'real.yaml'), 'name: real\ntimestamp: t\n');
  // A file that sits OUTSIDE the sessions dir (sibling of it) — the traversal target.
  secretOutside = path.join(configDir, 'secret.yaml');
  fs.writeFileSync(secretOutside, 'name: secret\n');
});

afterEach(() => {
  fs.rmSync(configDir, { recursive: true, force: true });
});

describe('loadSession — containment', () => {
  it('loads a legitimate session file inside the sessions dir', () => {
    expect(sessionService.loadSession('real.yaml')).toMatchObject({ name: 'real' });
  });

  it('returns null for a missing-but-contained filename (not a hard reject)', () => {
    expect(sessionService.loadSession('nope.yaml')).toBeNull();
  });

  it('rejects a traversal escaping the sessions dir', () => {
    expect(() => sessionService.loadSession('../secret.yaml')).toThrow(
      /escapes the sessions directory/,
    );
    // The out-of-tree file is untouched and unread.
    expect(fs.existsSync(secretOutside)).toBe(true);
  });

  it('rejects an absolute path', () => {
    expect(() => sessionService.loadSession('/etc/passwd')).toThrow(
      /escapes the sessions directory/,
    );
  });
});

describe('deleteSession — containment', () => {
  it('deletes a legitimate session file', () => {
    sessionService.deleteSession('real.yaml');
    expect(fs.existsSync(path.join(sessionsDir, 'real.yaml'))).toBe(false);
  });

  it('rejects a traversal and leaves the out-of-tree file intact', () => {
    expect(() => sessionService.deleteSession('../secret.yaml')).toThrow(
      /escapes the sessions directory/,
    );
    expect(fs.existsSync(secretOutside)).toBe(true);
  });

  it('is a no-op (no throw) for a missing-but-contained filename', () => {
    expect(() => sessionService.deleteSession('nope.yaml')).not.toThrow();
  });
});
