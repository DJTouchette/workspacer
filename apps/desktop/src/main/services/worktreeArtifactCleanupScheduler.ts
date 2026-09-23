import * as os from 'node:os';
import * as path from 'node:path';
import { configService } from './configService';
import { PORTS } from '../lib/daemonUtils';
import { cleanupWorktreeArtifacts, loadWorktreeCleanupState } from './worktreeArtifactCleanup';

export const ARTIFACT_CLEANUP_START_DELAY_MS = 60_000;
export const ARTIFACT_CLEANUP_INTERVAL_MS = 15 * 60_000;
export interface ArtifactCleanupConfig {
  agents?: {
    worktreeRoot?: string;
    artifactCleanup?: { enabled?: boolean; minAgeHours?: number };
  };
}

let cleanupInFlight = false;

/** Imported by Electron and the headless companion; never starts on import. */
export function startWorktreeArtifactCleanup(
  options: {
    remote?: boolean;
    getConfig?: () => ArtifactCleanupConfig;
    daemonUrl?: () => string;
  } = {},
): () => void {
  if (options.remote) return () => {};
  let stopped = false;
  let interval: ReturnType<typeof setInterval> | undefined;
  const run = async () => {
    if (stopped || cleanupInFlight) return;
    cleanupInFlight = true;
    try {
      const config = (options.getConfig ?? (() => configService.getConfig()))();
      const policy = config.agents?.artifactCleanup;
      if (policy?.enabled !== undefined && policy.enabled !== true) return;
      const minAgeHours = policy?.minAgeHours ?? 1;
      if (!Number.isFinite(minAgeHours) || minAgeHours < 0)
        throw new Error('agents.artifactCleanup.minAgeHours must be a nonnegative number');
      const configuredRoot = config.agents?.worktreeRoot?.trim();
      const root = configuredRoot
        ? path.resolve(configuredRoot)
        : path.join(os.homedir(), '.workspacer', 'worktrees');
      const daemonUrl = options.daemonUrl?.() ?? `http://127.0.0.1:${PORTS.claudemonApi}`;
      const report = await cleanupWorktreeArtifacts({
        root,
        apply: true,
        minAgeMs: minAgeHours * 60 * 60_000,
        loadState: () => {
          if (stopped) return Promise.reject(new Error('Artifact cleanup stopped'));
          return loadWorktreeCleanupState(daemonUrl);
        },
      });
      if (report.artifacts.length || report.errors.length || report.skipped.length)
        console.warn('[worktree-artifact-cleanup]', JSON.stringify(report));
    } catch (error) {
      if (!stopped) console.warn('[worktree-artifact-cleanup] unavailable', error);
    } finally {
      cleanupInFlight = false;
    }
  };
  const startup = setTimeout(() => {
    if (stopped) return;
    void run();
    interval = setInterval(() => void run(), ARTIFACT_CLEANUP_INTERVAL_MS);
    interval.unref?.();
  }, ARTIFACT_CLEANUP_START_DELAY_MS);
  startup.unref?.();
  return () => {
    stopped = true;
    clearTimeout(startup);
    if (interval) clearInterval(interval);
  };
}
