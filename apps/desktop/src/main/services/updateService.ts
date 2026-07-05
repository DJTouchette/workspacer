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
 */

import { app, BrowserWindow, dialog } from 'electron';
import { autoUpdater, type UpdateInfo } from 'electron-updater';
import { configService } from './configService';

/** How often to re-check the release feed after the startup check. */
const CHECK_INTERVAL_MS = 4 * 60 * 60 * 1000; // 4 hours

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

    // Download in the background as soon as an update is found; we prompt the
    // user only at the install (restart) step, matching the product choice.
    autoUpdater.autoDownload = true;
    autoUpdater.autoInstallOnAppQuit = false;
    autoUpdater.channel = channel;
    // electron-updater logs to console by default via its own logger; keep our
    // own breadcrumbs so update activity shows up in the app's log file.
    autoUpdater.on('checking-for-update', () => console.log('[updateService] checking for update'));
    autoUpdater.on('update-available', (info: UpdateInfo) =>
      console.log(`[updateService] update available: ${info.version} (downloading)`),
    );
    autoUpdater.on('update-not-available', () => console.log('[updateService] no update available'));
    autoUpdater.on('download-progress', (p) =>
      console.log(`[updateService] downloading ${Math.round(p.percent)}%`),
    );
    autoUpdater.on('update-downloaded', (info: UpdateInfo) => this.onDownloaded(info));
    // Errors (offline, unsigned-mac refusal, feed 404 before the first release,
    // …) are non-fatal by design — log at warn, never surface a dialog.
    autoUpdater.on('error', (err: Error) => {
      console.warn(`[updateService] updater error (non-fatal): ${err?.message ?? err}`);
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
