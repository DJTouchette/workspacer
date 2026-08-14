/**
 * Downloaded project icons. Two things matter here: what we accept off the
 * network, and what the `workspacer-icon://` handler is willing to serve — the
 * filename it resolves comes from config.yaml, which is a file a user (or
 * anything that can write it) edits by hand.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import * as http from 'http';

const h = vi.hoisted(() => {
  const nodeFs = require('fs') as typeof import('fs');
  const nodeOs = require('os') as typeof import('os');
  const nodePath = require('path') as typeof import('path');
  return { configDir: nodeFs.mkdtempSync(nodePath.join(nodeOs.tmpdir(), 'wks-icon-cfg-')) };
});
vi.mock('./configService', () => ({ getConfigDir: () => h.configDir }));

import {
  downloadProjectIcon,
  resolveProjectIcon,
  projectIconsDir,
  mimeForIcon,
} from './projectIcons';

// A 1x1 PNG — the smallest thing that is genuinely a PNG.
const PNG = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==',
  'base64',
);

let server: http.Server;
let base = '';
beforeEach(async () => {
  h.configDir = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-icon-cfg-'));
  server = http.createServer((req, res) => {
    // Path only — a query string must not change which fixture is served.
    const url = (req.url || '/').split('?')[0];
    if (url === '/icon.png') {
      res.writeHead(200, { 'Content-Type': 'image/png' });
      return res.end(PNG);
    }
    if (url === '/page.html') {
      res.writeHead(200, { 'Content-Type': 'text/html' });
      return res.end('<html>not an image</html>');
    }
    if (url === '/huge.png') {
      res.writeHead(200, { 'Content-Type': 'image/png' });
      return res.end(Buffer.alloc(3 * 1024 * 1024));
    }
    if (url === '/empty.png') {
      res.writeHead(200, { 'Content-Type': 'image/png' });
      return res.end(Buffer.alloc(0));
    }
    res.writeHead(404);
    res.end('nope');
  });
  await new Promise<void>((r) => server.listen(0, '127.0.0.1', r));
  base = `http://127.0.0.1:${(server.address() as any).port}`;
});
afterEach(async () => {
  await new Promise<void>((r) => server.close(() => r()));
  fs.rmSync(h.configDir, { recursive: true, force: true });
});

describe('downloadProjectIcon', () => {
  it('stores the image and returns a filename inside the icons dir', async () => {
    const { file } = await downloadProjectIcon(`${base}/icon.png`);
    expect(file).toMatch(/^[0-9a-f]{32}\.png$/);
    const full = path.join(projectIconsDir(), file);
    expect(fs.existsSync(full)).toBe(true);
    expect(fs.readFileSync(full).equals(PNG)).toBe(true);
  });

  it('is content-addressed, so the same icon twice is stored once', async () => {
    const a = await downloadProjectIcon(`${base}/icon.png`);
    const b = await downloadProjectIcon(`${base}/icon.png`);
    expect(a.file).toBe(b.file);
    expect(fs.readdirSync(projectIconsDir())).toHaveLength(1);
  });

  it('takes the extension from the RESPONSE, never from the URL', async () => {
    // Otherwise a URL ending .html (or .exe, or ../x) picks the filename of a
    // file the app later serves.
    const { file } = await downloadProjectIcon(`${base}/icon.png?x=evil.html`);
    expect(file.endsWith('.png')).toBe(true);
  });

  it('refuses anything that is not an image, with a reason', async () => {
    await expect(downloadProjectIcon(`${base}/page.html`)).rejects.toThrow(/not an image/i);
  });

  it('refuses non-http schemes — file: would read anything the app can', async () => {
    await expect(downloadProjectIcon('file:///etc/passwd')).rejects.toThrow(/http/i);
    await expect(downloadProjectIcon('data:image/png;base64,AAAA')).rejects.toThrow(/http/i);
  });

  it('reports a bad URL, an HTTP error, an empty body and an oversized one', async () => {
    await expect(downloadProjectIcon('not a url')).rejects.toThrow(/not a URL/i);
    await expect(downloadProjectIcon(`${base}/missing.png`)).rejects.toThrow(/404/);
    await expect(downloadProjectIcon(`${base}/empty.png`)).rejects.toThrow(/empty/i);
    await expect(downloadProjectIcon(`${base}/huge.png`)).rejects.toThrow(/2 MB/i);
  });
});

describe('resolveProjectIcon — what the protocol handler will serve', () => {
  it('resolves a stored file', async () => {
    const { file } = await downloadProjectIcon(`${base}/icon.png`);
    expect(resolveProjectIcon(file)).toBe(path.join(projectIconsDir(), file));
  });

  it('refuses to escape the icons directory', () => {
    // The filename comes from config.yaml. Serving whatever a config key names
    // would make this handler an arbitrary file reader — the sibling of that
    // directory is the config dir, which holds remote-token.
    for (const evil of [
      '../config.yaml',
      '../../.ssh/id_rsa',
      '/etc/passwd',
      'sub/dir.png',
      '..\\config.yaml',
      '.',
      '..',
      '',
    ]) {
      expect(resolveProjectIcon(evil), `${evil} must be refused`).toBeNull();
    }
  });

  it('returns null for a name that is well-formed but absent', () => {
    expect(resolveProjectIcon('deadbeef.png')).toBeNull();
  });
});

describe('mimeForIcon', () => {
  it('serves each stored extension as its real type', () => {
    expect(mimeForIcon('a.png')).toBe('image/png');
    expect(mimeForIcon('a.svg')).toBe('image/svg+xml');
    expect(mimeForIcon('a.ico')).toBe('image/x-icon');
    expect(mimeForIcon('a.weird')).toBe('application/octet-stream');
  });
});
