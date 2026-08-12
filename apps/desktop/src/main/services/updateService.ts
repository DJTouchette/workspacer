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
import { openExternalUrl } from './hubCapabilities';
import { IPC } from '../shared/ipcChannels';

/** How often to re-check the release feed after the startup check. */
const CHECK_INTERVAL_MS = 4 * 60 * 60 * 1000; // 4 hours

/**
 * This app's releases page. Owner/repo mirror electron-builder.yml's publish
 * block; the nightly feed and the "What's new" link are both cut from it, so
 * there is one literal to change if the repo ever moves.
 */
const RELEASES_URL = 'https://github.com/DJTouchette/workspacer/releases';

/**
 * A plain semver, optionally with a prerelease tail (`0.149.0`,
 * `0.149.0-nightly.20260811.abc1234`). Nothing else — see releaseNotesUrl.
 */
const VERSION_RE = /^\d+\.\d+\.\d+(?:-[A-Za-z0-9.]+)?$/;

/**
 * The release page for a version offered by the update feed.
 *
 * `info.version` is a value the FEED supplies, and it is string-concatenated
 * into a URL we then hand to the OS browser. `new URL()` collapses `..`
 * segments, so an unchecked version of the form `v../../someone/else` resolves
 * to a different repository's page entirely — the user clicks "What's new" in
 * our own trusted dialog and lands on release notes an attacker wrote for the
 * build they are about to install. Anything that is not a bare version falls
 * back to the releases index, which is never wrong, only less specific.
 *
 * A nightly's notes are the rolling `nightly` prerelease, not a `vX.Y.Z` tag —
 * that tag holds the last STABLE release, which is not what is being installed.
 */
export function releaseNotesUrl(version: unknown): string {
  if (typeof version !== 'string' || !VERSION_RE.test(version)) {
    console.warn(
      `[updateService] unexpected version ${JSON.stringify(version)} — linking the releases index`,
    );
    return RELEASES_URL;
  }
  if (version.includes('-nightly')) return `${RELEASES_URL}/tag/nightly`;
  return `${RELEASES_URL}/tag/v${version}`;
}

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

/**
 * A bare channel name and nothing else: it must start alphanumeric and may then
 * carry dots, dashes and underscores. No slashes, so `../` can't appear and a
 * leading `//` can't either.
 */
const CHANNEL_RE = /^[a-z0-9][a-z0-9._-]*$/;

/**
 * Clamp `updates.channel` to a channel NAME before it reaches
 * electron-updater.
 *
 * The channel is string-concatenated into the update feed URL (it becomes
 * `<channel>.yml` on the provider's base URL), and it is writable by anything
 * that can call `config.save` — which includes the hub bus and the MCP facade,
 * not just the settings UI. A value like `../../someone-else/repo/releases/latest`
 * therefore relocates the updater onto a feed an attacker controls, and the next
 * "Workspacer 99.0.0 is ready to install" dialog — the app's own, trusted
 * dialog — hands them code execution as the user, persistently. A
 * scheme-relative `//host/path` does the same thing at the host level.
 *
 * Anything that isn't a plain name falls back to 'latest' with a warning rather
 * than failing the update check: a malformed channel is a config mistake as
 * often as an attack, and the stable feed is the safe default either way.
 */
export function sanitizeUpdateChannel(raw: unknown): string {
  if (typeof raw !== 'string' || !raw) return 'latest';
  if (!CHANNEL_RE.test(raw)) {
    console.warn(
      `[updateService] ignoring invalid updates.channel ${JSON.stringify(raw)} — using 'latest'`,
    );
    return 'latest';
  }
  return raw;
}

/** Read + normalise the `updates` config block (absent ⇒ enabled/latest). */
function readUpdatesConfig(): UpdatesConfig {
  const u = ((configService.getConfig() as any).updates ?? {}) as Partial<UpdatesConfig>;
  return {
    enabled: u.enabled !== false,
    channel: sanitizeUpdateChannel(u.channel),
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
        url: `${RELEASES_URL}/download/nightly`,
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
    if (!this.win || this.win.isDestroyed()) return;
    if (this.promptOpen) return;
    this.promptOpen = true;

    try {
      // A loop, not one question: "What's new" is not an ANSWER to "restart
      // now?". Opening the notes and treating that as a decision would silently
      // count reading as declining, and the running build cannot show the notes
      // itself — changelog.generated.ts is baked at ITS build time and predates
      // the release being offered, so the notes have to come from the release
      // page. Each turn of the loop is a fresh user click; the destroyed-window
      // check is what stops it spinning if the window goes away underneath.
      for (;;) {
        const win = this.win;
        if (!win || win.isDestroyed()) return;

        const { response } = await dialog.showMessageBox(win, {
          type: 'info',
          buttons: ['Restart now', "What's new", 'Later'],
          defaultId: 0,
          cancelId: 2,
          title: 'Update ready',
          message: `Workspacer ${info.version} is ready to install.`,
          detail: 'Restart to apply the update. Your session is saved on quit.',
        });

        if (response === 1) {
          const opened = await openExternalUrl(releaseNotesUrl(info.version));
          if (!opened.ok) {
            console.warn(`[updateService] could not open the release notes: ${opened.error}`);
          }
          continue; // back to the same choice
        }
        if (response === 0) {
          // Let the normal quit path save the session, then swap in the update.
          autoUpdater.quitAndInstall();
        }
        return;
      }
    } catch (err) {
      console.warn(`[updateService] install prompt failed: ${(err as Error)?.message ?? err}`);
    } finally {
      this.promptOpen = false;
    }
  }
}

export const updateService = new UpdateService();
