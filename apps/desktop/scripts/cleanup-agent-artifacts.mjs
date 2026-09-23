#!/usr/bin/env node
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import yaml from 'js-yaml';
import { build } from 'esbuild';

const usage = `Reclaim generated artifacts from stopped agent worktrees.

  npm run cleanup:agents -- [--apply] [--root PATH] [--min-age-hours HOURS]
                           [--daemon-url URL]

Dry-run is the default. --apply removes only the core's eligible artifacts.
The configured agents.worktreeRoot and artifactCleanup.minAgeHours are honored;
defaults are ~/.workspacer/worktrees and 1 hour. A compatible running claudemon
is required for deletion. Source files, worktree branches and dependency targets
outside the worktree are preserved. Output is a JSON report of paths, bytes,
skipped worktrees and errors.
`;

export function parseCleanupArgs(args) {
  const options = { apply: false };
  for (let i = 0; i < args.length; i++) {
    const arg = args[i];
    if (arg === '--help' || arg === '-h') options.help = true;
    else if (arg === '--apply') options.apply = true;
    else if (['--root', '--min-age-hours', '--daemon-url'].includes(arg)) {
      const value = args[++i];
      if (!value || value.startsWith('--')) throw new Error(`${arg} requires a value`);
      if (arg === '--root') options.root = value;
      if (arg === '--daemon-url') options.daemonUrl = value;
      if (arg === '--min-age-hours') options.minAgeHours = Number(value);
    } else throw new Error(`Unknown argument: ${arg}`);
  }
  if (
    options.minAgeHours !== undefined &&
    (!Number.isFinite(options.minAgeHours) || options.minAgeHours < 0)
  )
    throw new Error('--min-age-hours must be a nonnegative number');
  return options;
}

export function cleanupConfigPath(
  env = process.env,
  platform = process.platform,
  home = os.homedir(),
) {
  const base =
    platform === 'win32'
      ? env.APPDATA || path.join(home, 'AppData', 'Roaming')
      : env.XDG_CONFIG_HOME || path.join(home, '.config');
  return path.join(base, 'workspacer', 'config.yaml');
}

export function resolveCleanupOptions(options, config, home = os.homedir()) {
  if (!config || typeof config !== 'object' || Array.isArray(config))
    throw new Error('config.yaml must contain a configuration object');
  const configuredRoot = config?.agents?.worktreeRoot;
  if (configuredRoot !== undefined && typeof configuredRoot !== 'string')
    throw new Error('agents.worktreeRoot must be a string');
  const minAgeHours = options.minAgeHours ?? config?.agents?.artifactCleanup?.minAgeHours ?? 1;
  if (typeof minAgeHours !== 'number' || !Number.isFinite(minAgeHours) || minAgeHours < 0)
    throw new Error('agents.artifactCleanup.minAgeHours must be a nonnegative number');
  return {
    root: path.resolve(
      options.root || configuredRoot?.trim() || path.join(home, '.workspacer', 'worktrees'),
    ),
    apply: options.apply,
    minAgeMs: minAgeHours * 60 * 60_000,
  };
}

async function main(args) {
  const options = parseCleanupArgs(args);
  if (options.help) {
    console.log(usage);
    return;
  }
  let config = {};
  try {
    config = yaml.load(fs.readFileSync(cleanupConfigPath(), 'utf8')) ?? {};
  } catch (error) {
    if (error.code !== 'ENOENT') throw error;
  }
  const cleanupOptions = resolveCleanupOptions(options, config);
  const daemonUrl =
    options.daemonUrl ||
    `http://127.0.0.1:${7891 + (Number(process.env.WORKSPACER_PORT_OFFSET) || 0)}`;
  const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-artifact-cleanup-'));
  try {
    const bundle = path.join(temp, 'cleanup.cjs');
    await build({
      entryPoints: [
        fileURLToPath(new URL('../src/main/services/worktreeArtifactCleanup.ts', import.meta.url)),
      ],
      outfile: bundle,
      bundle: true,
      platform: 'node',
      format: 'cjs',
      target: 'node22',
    });
    const { cleanupWorktreeArtifacts, loadWorktreeCleanupState } = createRequire(import.meta.url)(
      bundle,
    );
    const report = await cleanupWorktreeArtifacts({
      ...cleanupOptions,
      loadState: () => loadWorktreeCleanupState(daemonUrl),
    });
    console.log(JSON.stringify(report, null, 2));
    if (report.errors.length) process.exitCode = 1;
  } finally {
    fs.rmSync(temp, { recursive: true, force: true });
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2)).catch((error) => {
    console.error(`[cleanup:agents] ${error.message ?? error}`);
    process.exitCode = 1;
  });
}
