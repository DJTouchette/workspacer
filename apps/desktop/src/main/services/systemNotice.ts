/**
 * System notices — surface main-process failures (a daemon that won't start, a
 * crash-loop that gave up, a missing provider CLI) to the user as an in-app
 * banner instead of dying silently in the console. Without this, a failed
 * claudemon/hub leaves the UI blank with no explanation (the #1 "the app is
 * broken and I don't know why" trap for a packaged build).
 *
 * Notices raised before the renderer has finished loading are buffered and
 * flushed on `did-finish-load`, so a fast startup failure isn't lost.
 */
import { BrowserWindow } from 'electron';
import { IPC } from '../shared/ipcChannels';

export interface SystemNotice {
  level: 'error' | 'warn' | 'info';
  /** Short headline, e.g. "claudemon failed to start". */
  title: string;
  /** Optional detail / remediation hint. */
  detail?: string;
  /** Stable key so repeated raises of the same condition replace, not stack. */
  key?: string;
}

let win: BrowserWindow | null = null;
let ready = false;
const pending: SystemNotice[] = [];

/** Wire the window that receives notices. Buffers until its renderer loads. */
export function setNoticeWindow(w: BrowserWindow): void {
  win = w;
  ready = false;
  w.webContents.on('did-finish-load', () => {
    ready = true;
    flush();
  });
}

function flush(): void {
  if (!ready || !win || win.isDestroyed()) return;
  for (const notice of pending.splice(0)) {
    win.webContents.send(IPC.SYSTEM_NOTICE, notice);
  }
}

/** Raise a notice (also logged). Delivered now if the renderer is up, else queued. */
export function notifySystem(notice: SystemNotice): void {
  const line = `[system-notice] ${notice.level}: ${notice.title}${notice.detail ? ` — ${notice.detail}` : ''}`;
  if (notice.level === 'error') console.error(line);
  else console.warn(line);
  pending.push(notice);
  flush();
}
