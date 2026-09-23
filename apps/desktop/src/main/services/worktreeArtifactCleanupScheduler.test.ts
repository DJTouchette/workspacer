import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
vi.mock('./configService', () => ({ configService: { getConfig: vi.fn() } }));
vi.mock('./worktreeArtifactCleanup', () => ({
  cleanupWorktreeArtifacts: vi.fn(),
  loadWorktreeCleanupState: vi.fn(),
}));
import {
  cleanupWorktreeArtifacts,
  loadWorktreeCleanupState,
  type CleanupReport,
} from './worktreeArtifactCleanup';
import {
  startWorktreeArtifactCleanup,
  ARTIFACT_CLEANUP_START_DELAY_MS,
  ARTIFACT_CLEANUP_INTERVAL_MS,
  type ArtifactCleanupConfig,
} from './worktreeArtifactCleanupScheduler';
const report: CleanupReport = {
  root: '/worktrees',
  apply: true,
  scannedWorktrees: 0,
  removedBytes: 0,
  reclaimableBytes: 0,
  artifacts: [],
  skipped: [],
  errors: [],
};
const stops: Array<() => void> = [];
beforeEach(() => {
  vi.useFakeTimers();
  vi.mocked(cleanupWorktreeArtifacts).mockReset().mockResolvedValue(report);
  vi.mocked(loadWorktreeCleanupState)
    .mockReset()
    .mockResolvedValue({ maintenanceSupported: true, sessions: [] });
});
afterEach(() => {
  for (const stop of stops.splice(0)) stop();
  vi.restoreAllMocks();
  vi.useRealTimers();
});
describe('automatic worktree artifact cleanup', () => {
  it('starts after the startup delay and uses current configuration each interval', async () => {
    let config: ArtifactCleanupConfig = {
      agents: { worktreeRoot: '/first', artifactCleanup: { enabled: true, minAgeHours: 2 } },
    };
    stops.push(
      startWorktreeArtifactCleanup({
        getConfig: () => config,
        daemonUrl: () => 'http://127.0.0.1:7991',
      }),
    );
    await vi.advanceTimersByTimeAsync(ARTIFACT_CLEANUP_START_DELAY_MS - 1);
    expect(cleanupWorktreeArtifacts).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(cleanupWorktreeArtifacts).toHaveBeenCalledWith(
      expect.objectContaining({ root: '/first', apply: true, minAgeMs: 7200000 }),
    );
    await vi.mocked(cleanupWorktreeArtifacts).mock.calls[0][0].loadState();
    expect(loadWorktreeCleanupState).toHaveBeenCalledWith('http://127.0.0.1:7991');
    config = {
      agents: { worktreeRoot: '/second', artifactCleanup: { enabled: true, minAgeHours: 4 } },
    };
    await vi.advanceTimersByTimeAsync(ARTIFACT_CLEANUP_INTERVAL_MS);
    expect(cleanupWorktreeArtifacts).toHaveBeenLastCalledWith(
      expect.objectContaining({ root: '/second', minAgeMs: 14400000 }),
    );
  });
  it('skips remote clients and honors disabling without needing a restart', async () => {
    const getConfig = vi.fn(() => ({ agents: { artifactCleanup: { enabled: false } } }));
    stops.push(startWorktreeArtifactCleanup({ remote: true, getConfig }));
    expect(vi.getTimerCount()).toBe(0);
    stops.push(startWorktreeArtifactCleanup({ getConfig }));
    await vi.advanceTimersByTimeAsync(ARTIFACT_CLEANUP_START_DELAY_MS);
    expect(cleanupWorktreeArtifacts).not.toHaveBeenCalled();
    getConfig.mockReturnValue({ agents: { artifactCleanup: { enabled: true } } });
    await vi.advanceTimersByTimeAsync(ARTIFACT_CLEANUP_INTERVAL_MS);
    expect(cleanupWorktreeArtifacts).toHaveBeenCalledTimes(1);
    getConfig.mockReturnValue({ agents: { artifactCleanup: { enabled: false } } });
    await vi.advanceTimersByTimeAsync(ARTIFACT_CLEANUP_INTERVAL_MS);
    expect(cleanupWorktreeArtifacts).toHaveBeenCalledTimes(1);
  });
  it('does not overlap runs and shutdown prevents further state reads or timers', async () => {
    let finish!: (value: CleanupReport) => void;
    vi.mocked(cleanupWorktreeArtifacts).mockImplementation(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
    const stop = startWorktreeArtifactCleanup({ getConfig: () => ({}) });
    stops.push(stop);
    await vi.advanceTimersByTimeAsync(
      ARTIFACT_CLEANUP_START_DELAY_MS + 2 * ARTIFACT_CLEANUP_INTERVAL_MS,
    );
    expect(cleanupWorktreeArtifacts).toHaveBeenCalledTimes(1);
    stop();
    await expect(vi.mocked(cleanupWorktreeArtifacts).mock.calls[0][0].loadState()).rejects.toThrow(
      'stopped',
    );
    finish(report);
    await vi.advanceTimersByTimeAsync(ARTIFACT_CLEANUP_INTERVAL_MS);
    expect(cleanupWorktreeArtifacts).toHaveBeenCalledTimes(1);
    expect(vi.getTimerCount()).toBe(0);
  });
  it('retries failures on the next interval and logs on stderr', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    vi.mocked(cleanupWorktreeArtifacts).mockRejectedValueOnce(new Error('daemon unavailable'));
    stops.push(startWorktreeArtifactCleanup({ getConfig: () => ({}) }));
    await vi.advanceTimersByTimeAsync(ARTIFACT_CLEANUP_START_DELAY_MS);
    expect(warn).toHaveBeenCalledWith('[worktree-artifact-cleanup] unavailable', expect.any(Error));
    await vi.advanceTimersByTimeAsync(ARTIFACT_CLEANUP_INTERVAL_MS);
    expect(cleanupWorktreeArtifacts).toHaveBeenCalledTimes(2);
  });
});
