#!/usr/bin/env node
// One backend executable owns the bus, MCP facade and optional embedded engine.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const manifest = path.join(root, 'services/hub-rs/Cargo.toml');
const metadata = spawnSync('cargo', ['metadata', '--locked', '--no-deps', '--format-version=1', '--manifest-path', manifest], { cwd: root, encoding: 'utf8' });
if (metadata.status !== 0) throw new Error(metadata.stderr || 'Cargo metadata failed');
const target = JSON.parse(metadata.stdout).target_directory;
const build = spawnSync('cargo', ['build', '--locked', '--release', '--manifest-path', manifest, '--bin', 'workspacer-rust'], { cwd: root, stdio: 'inherit' });
if (build.status !== 0) process.exit(build.status ?? 1);
const name = process.platform === 'win32' ? 'workspacer-rust.exe' : 'workspacer-rust';
const source = path.join(target, 'release', name);
const canonical = path.join(root, 'services/hub-rs/target/release', name);
// Electron packaging has a stable resource path even with a shared Cargo cache.
if (path.resolve(source) !== path.resolve(canonical)) {
  fs.mkdirSync(path.dirname(canonical), { recursive: true });
  fs.copyFileSync(source, canonical);
  if (process.platform !== 'win32') fs.chmodSync(canonical, 0o755);
}
