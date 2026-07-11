/**
 * "Install workspacer command" — runs the bundled `workspacer` CLI's own
 * `install-cli` subcommand, which symlinks/copies the binary onto PATH
 * (/usr/local/bin → ~/.local/bin on Unix; %LOCALAPPDATA%\workspacer\bin plus
 * printed PATH instructions on Windows). All install policy lives in the CLI
 * itself (services/hub/cmd/workspacer/install.go) — we only spawn it and
 * surface what it printed.
 *
 * Binary resolution (mirrors hubDaemon.ts):
 *   - dev (ELECTRON_DEV=1): <repo>/services/hub/workspacer[.exe] (`npm run build:cli`)
 *   - packaged:             <resourcesPath>/hub/workspacer[.exe]
 */

import * as fs from 'fs';
import * as path from 'path';
import { execFile } from 'child_process';
import { app } from 'electron';
import { notifySystem } from './systemNotice';

/** How long the install may take before we give up (it's a local symlink/copy). */
const INSTALL_TIMEOUT_MS = 15_000;

export interface CliInstallResult {
  ok: boolean;
  /** What the CLI printed: the install destination on success (plus PATH
   *  instructions when the target dir isn't on PATH), or the error. */
  message: string;
}

function exeName(): string {
  return process.platform === 'win32' ? 'workspacer.exe' : 'workspacer';
}

/** Resolve the bundled `workspacer` CLI for the current run mode. It ships in
 *  hub/ next to hub/brain/claudemon so its sibling-first daemon resolution
 *  works from the packaged layout with zero flags. */
export function workspacerCliPath(): string {
  if (process.env.ELECTRON_DEV || !app.isPackaged) {
    return path.join(app.getAppPath(), '..', '..', 'services', 'hub', exeName());
  }
  return path.join(process.resourcesPath, 'hub', exeName());
}

/**
 * Run `workspacer install-cli` and report the outcome. Also raises a system
 * notice (banner) so the command palette path gets visible feedback without
 * its own result UI; the Settings row additionally shows the message inline.
 */
export function installWorkspacerCli(): Promise<CliInstallResult> {
  const bin = workspacerCliPath();
  if (!fs.existsSync(bin)) {
    const message = `workspacer CLI not found at ${bin} (dev: run \`npm run build:cli\`)`;
    notifySystem({
      level: 'error',
      key: 'cli-install',
      title: 'Install CLI failed',
      detail: message,
    });
    return Promise.resolve({ ok: false, message });
  }
  return new Promise((resolve) => {
    execFile(bin, ['install-cli'], { timeout: INSTALL_TIMEOUT_MS }, (err, stdout, stderr) => {
      const out = `${stdout ?? ''}${stderr ?? ''}`.trim();
      const result: CliInstallResult = err
        ? { ok: false, message: out || String(err.message ?? err) }
        : { ok: true, message: out || 'workspacer command installed' };
      notifySystem({
        level: result.ok ? 'info' : 'error',
        key: 'cli-install',
        title: result.ok ? 'workspacer command installed' : 'Install CLI failed',
        detail: result.message,
      });
      resolve(result);
    });
  });
}
