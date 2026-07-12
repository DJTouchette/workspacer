/**
 * In-app auto-update via electron-updater + the latest published GitHub Release.
 *
 * Only runs in a packaged production build (`app.isPackaged`) — in dev there is
 * no code signature / update feed and electron-updater would throw, so we no-op
 * with a log. On startup and every ~4h it checks the GitHub Release feed,
 * downloads a newer build in the background, then prompts (using the same dialog
 * conventions as the rest of the app) before `quitAndInstall`.
 *
 * Gating:
 *  - `updates.enabled` (config, default true) is the master switch.
 *  - `updates.channel` (default 'latest') selects the release channel.
 *  - macOS refuses to apply UNSIGNED updates: electron-updater emits an `error`
 *    rather than an `update-available`. We treat every updater error as
 *    non-fatal and log-only (never a user-facing dialog), so an unsigned mac
 *    build degrades silently and the flow lights up on its own once signing +
 *    a zip target land — no code change required.
 *  - Nightly builds (version contains `-nightly`) update from the rolling
 *    `nightly` prerelease via the generic provider (see wire()); stable and
 *    nightly feeds never cross.
 */

import { app, BrowserWindow, dialog } from 'electron';
import { autoUpdater, type UpdateInfo } from 'electron-updater';
import { configService } from './configService';
import { IPC } from '../shared/ipcChannels';

/** How often to re-check the release feed after the startup check. */
const CHECK_INTERVAL_MS = 4 * 60 * 60 * 1000; // 4 hours

/** Renderer-visible update state (pushed on UPDATES_STATUS; see ipcChannels). */
export interface UpdateStatus {
  state:
    | 'unsupported' // dev build / web mirror — no update feed
    | 'disabled' // updates.enabled=false in config
    | 'idle' // checked, nothing newer
    | 'checking'
    | 'downloading'
    | 'downloaded' // ready — install restarts into it
    | 'error';
  /** The newer version, once known (downloading/downloaded). */
  version?: string;
  /** Download progress 0–100 while downloading. */
  percent?: number;
  /** The running app's version. */
  current: string;
  error?: string;
}

interface UpdatesConfig {
  /** Master switch for in-app auto-update. Default true. */
  enabled: boolean;
  /** Release channel electron-updater reads ('latest', 'beta', …). */
  channel: string;
}

/** Read + normalise the `updates` config block (absent ⇒ enabled/latest). */
function readUpdatesConfig(): UpdatesConfig {
  const u = ((configService.getConfig() as any).updates ?? {}) as Partial<UpdatesConfig>;
  return {
    enabled: u.enabled !== false,
    channel: typeof u.channel === 'string' && u.channel ? u.channel : 'latest',
  };
}

class UpdateService {
  private win: BrowserWindow | null = null;
  private timer: ReturnType<typeof setInterval> | null = null;
  private wired = false;
  /** Guard so overlapping checks (startup + interval) don't stack dialogs. */
  private promptOpen = false;
  private status: UpdateStatus = {
    state: app.isPackaged ? 'idle' : 'unsupported',
    current: app.getVersion(),
  };

  /** Current status, for the renderer's initial pull. */
  getStatus(): UpdateStatus {
    return this.status;
  }

  /** Transition + push to the renderer so the palette/overview stay live. */
  private setStatus(patch: Partial<UpdateStatus>): void {
    this.status = { ...this.status, ...patch };
    const win = this.win;
    try {
      if (win && !win.isDestroyed()) {
        win.webContents?.send(IPC.UPDATES_STATUS, this.status);
      }
    } catch {
      /* window mid-teardown — the renderer re-pulls on next mount */
    }
  }

  /**
   * Manual "check now" (palette). Works even when auto-update is disabled in
   * config — an explicit ask is explicit consent. No-op in dev/web.
   */
  async checkNow(): Promise<UpdateStatus> {
    if (!app.isPackaged) return this.status; // unsupported
    this.wire(readUpdatesConfig().channel);
    await this.check();
    return this.status;
  }

  /** Restart into a downloaded update (palette / overview banner). */
  installNow(): void {
    if (this.status.state !== 'downloaded') return;
    autoUpdater.quitAndInstall();
  }

  /**
   * Wire and start the updater. Safe to call once with the main window. No-ops
   * (with a log) outside a packaged build or when `updates.enabled` is false.
   */
  start(win: BrowserWindow): void {
    this.win = win;

    if (!app.isPackaged) {
      console.log('[updateService] dev build — auto-update disabled');
      return;
    }

    const cfg = readUpdatesConfig();
    if (!cfg.enabled) {
      console.log('[updateService] disabled via config (updates.enabled=false)');
      this.setStatus({ state: 'disabled' });
      return;
    }

    this.wire(cfg.channel);

    // Startup check, then a periodic re-check. `checkForUpdates` returns a
    // rejected promise on failure too (e.g. offline / unsigned mac); swallow it
    // so a failed check never bubbles as an unhandled rejection.
    void this.check();
    this.timer = setInterval(() => void this.check(), CHECK_INTERVAL_MS);
  }

  /** Stop the periodic check (called on shutdown). */
  stop(): void {
    if (this.timer) {
      clearInterval(this.timer);
      this.timer = null;
    }
  }

  /** Install the electron-updater listeners exactly once. */
  private wire(channel: string): void {
    if (this.wired) return;
    this.wired = true;

    // Nightly builds update from the rolling `nightly` prerelease instead of
    // the stable feed. The GitHub provider can't serve it — it parses release
    // tags as semver and the rolling tag is literally "nightly" — so nightlies
    // switch to the generic provider aimed at the tag's stable download URL
    // (where the workflow attaches latest*.yml). Stable installs never see
    // that feed: their provider resolves /releases/latest, which GitHub keeps
    // free of prereleases.
    const isNightly = app.getVersion().includes('-nightly');
    if (isNightly) {
      autoUpdater.setFeedURL({
        provider: 'generic',
        // Owner/repo mirror electron-builder.yml's publish block.
        url: 'https://github.com/DJTouchette/workspacer/releases/download/nightly',
        // GitHub's release CDN 501s multipart Range requests; single-range
        // gets a 206, which keeps blockmap differential downloads working
        // (electron-updater's own GitHub provider forces this too).
        useMultipleRangeRequest: false,
      });
      // Rolling-nightly stamps aren't guaranteed monotonic across stamp-format
      // changes; nightlies trust the feed rather than semver ordering.
      autoUpdater.allowDowngrade = true;
      console.log('[updateService] nightly build — updating from the rolling nightly feed');
    }

    // Download in the background as soon as an update is found; we prompt the
    // user only at the install (restart) step, matching the product choice.
    autoUpdater.autoDownload = true;
    autoUpdater.autoInstallOnAppQuit = false;
    // The nightly feed only publishes latest*.yml — a configured channel like
    // 'beta' would make the generic provider request beta.yml, 404, and
    // silently kill nightly updates. Channels are a stable-feed concept.
    autoUpdater.channel = isNightly ? 'latest' : channel;
    // electron-updater logs to console by default via its own logger; keep our
    // own breadcrumbs so update activity shows up in the app's log file.
    autoUpdater.on('checking-for-update', () => {
      console.log('[updateService] checking for update');
      this.setStatus({ state: 'checking', error: undefined });
    });
    autoUpdater.on('update-available', (info: UpdateInfo) => {
      console.log(`[updateService] update available: ${info.version} (downloading)`);
      this.setStatus({ state: 'downloading', version: info.version, percent: 0 });
    });
    autoUpdater.on('update-not-available', () => {
      console.log('[updateService] no update available');
      this.setStatus({ state: 'idle', version: undefined, percent: undefined });
    });
    autoUpdater.on('download-progress', (p) => {
      console.log(`[updateService] downloading ${Math.round(p.percent)}%`);
      this.setStatus({ state: 'downloading', percent: Math.round(p.percent) });
    });
    autoUpdater.on('update-downloaded', (info: UpdateInfo) => {
      this.setStatus({ state: 'downloaded', version: info.version, percent: 100 });
      void this.onDownloaded(info);
    });
    // Errors (offline, unsigned-mac refusal, feed 404 before the first release,
    // …) are non-fatal by design — log at warn, never surface a dialog.
    autoUpdater.on('error', (err: Error) => {
      console.warn(`[updateService] updater error (non-fatal): ${err?.message ?? err}`);
      this.setStatus({ state: 'error', error: String(err?.message ?? err) });
    });
  }

  private async check(): Promise<void> {
    try {
      await autoUpdater.checkForUpdates();
    } catch (err) {
      // The 'error' listener already logs; catch here purely to keep this from
      // becoming an unhandled rejection.
      void err;
    }
  }

  /** A newer build is on disk — ask the user whether to restart into it now. */
  private async onDownloaded(info: UpdateInfo): Promise<void> {
    const win = this.win;
    if (!win || win.isDestroyed()) return;
    if (this.promptOpen) return;
    this.promptOpen = true;

    try {
      const { response } = await dialog.showMessageBox(win, {
        type: 'info',
        buttons: ['Restart now', 'Later'],
        defaultId: 0,
        cancelId: 1,
        title: 'Update ready',
        message: `Workspacer ${info.version} is ready to install.`,
        detail: 'Restart to apply the update. Your session is saved on quit.',
      });
      if (response === 0) {
        // Let the normal quit path save the session, then swap in the update.
        autoUpdater.quitAndInstall();
      }
    } catch (err) {
      console.warn(`[updateService] install prompt failed: ${(err as Error)?.message ?? err}`);
    } finally {
      this.promptOpen = false;
    }
  }
}

export const updateService = new UpdateService();
