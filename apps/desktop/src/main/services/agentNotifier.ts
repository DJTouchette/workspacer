/**
 * OS notifications + taskbar attention + in-app notification mirroring for
 * agent state changes.
 *
 * Driven from `claudeSessionStore` on every ambient-state transition. We fire
 * an OS notification when an agent transitions *into* a needs-you state
 * (approval / input) or *finishes* (working → idle), so you can babysit agents
 * you're not actively watching. Clicking a notification focuses the window and
 * tells the renderer to jump to that agent. The body carries the actual
 * context — the tool awaiting approval, the question text, the session cost —
 * so the notification is decidable without switching to the app.
 *
 * Every fired notification is also mirrored into the renderer's notification
 * center (NOTIFY_IN_APP push); other main-process producers (budget watcher,
 * hub `notifications.post` capability) reuse `postInApp`/`focusAgent` here so
 * there is exactly one owner of the main→renderer notification seam.
 *
 * Behaviour is config-driven (`notifications` block); the defaults match the
 * product choice: needs-you + done, only when unwatched, silent.
 */

import { randomUUID } from 'crypto';
import { BrowserWindow, Notification } from 'electron';
import { configService } from './configService';
import { appIconPath } from '../lib/appIcon';
import { IPC } from '../shared/ipcChannels';
import type { InAppNotification } from '../shared/ipcTypes';
import type { ClaudeSessionState, SessionAmbientState } from './claudeSessionStore';

const NEEDS_YOU: SessionAmbientState[] = ['waiting_approval', 'waiting_input'];
// 'background' counts as working so "finished" fires when the spawned work
// (workflow / background subagent) completes — background → idle — instead of
// when the parent turn ends mid-workflow.
const WORKING: SessionAmbientState[] = ['streaming', 'thinking', 'background'];

interface NotificationsConfig {
  enabled: boolean;
  /** Notify when an agent finishes (working → idle). */
  notifyDone: boolean;
  /** Suppress notifications for the agent you're currently looking at. */
  onlyWhenUnwatched: boolean;
  sound: boolean;
}

/** Human label for a session — its explicit label, else the cwd basename. */
function agentLabel(session: ClaudeSessionState): string {
  if (session.label) return session.label;
  const cwd = session.cwd;
  if (!cwd) return 'Agent';
  const parts = cwd.replace(/[/\\]+$/, '').split(/[/\\]/);
  return parts[parts.length - 1] || cwd;
}

function truncate(s: string, max: number): string {
  const line = s.replace(/\s+/g, ' ').trim();
  return line.length <= max ? line : line.slice(0, max - 1) + '…';
}

/** The one-line gist of a tool call, for approval notification bodies. Picks
 *  the field a human actually decides on (command, path, url, description). */
function summarizeToolInput(input: unknown): string | null {
  if (!input || typeof input !== 'object') return null;
  const d = input as Record<string, unknown>;
  if (typeof d.command === 'string' && d.command) return `$ ${d.command}`;
  if (typeof d.file_path === 'string' && d.file_path) return d.file_path;
  if (typeof d.url === 'string' && d.url) return d.url;
  if (typeof d.description === 'string' && d.description) return d.description;
  if (typeof d.pattern === 'string' && d.pattern) return d.pattern;
  return null;
}

/** Best-known cumulative session cost (statusLine is authoritative when live). */
function sessionCost(session: ClaudeSessionState): number {
  return session.statusLine?.costUSD ?? session.usage?.costUSD ?? 0;
}

class AgentNotifier {
  private mainWindow: BrowserWindow | null = null;
  /** sessionId the renderer currently has on screen (null if none/stopped). */
  private activeSessionId: string | null = null;
  /** In-app notifications raised before the renderer loaded, flushed on load. */
  private pendingInApp: InAppNotification[] = [];
  private rendererReady = false;
  /**
   * OS toasts currently shown. A Notification instance that gets garbage
   * collected stops emitting events — on Windows that manifests as toasts
   * whose clicks silently do nothing — so hold a reference until the OS
   * reports the toast closed.
   */
  private liveToasts = new Set<Notification>();

  setMainWindow(win: BrowserWindow): void {
    this.mainWindow = win;
    this.rendererReady = false;
    // Remove any prior focus listener from a previous window to prevent
    // duplicate handlers accumulating across setMainWindow calls.
    win.removeAllListeners('focus');
    // Stop the taskbar flashing as soon as the user looks at the window.
    win.on('focus', () => {
      try {
        win.flashFrame(false);
      } catch {
        /* noop */
      }
    });
    win.webContents.on('did-finish-load', () => {
      this.rendererReady = true;
      this.flushInApp();
    });
  }

  /** Renderer reports which agent session is currently visible. */
  setActiveSession(sessionId: string | null): void {
    this.activeSessionId = sessionId;
  }

  private cfg(): NotificationsConfig {
    const c = ((configService.getConfig() as any).notifications ??
      {}) as Partial<NotificationsConfig>;
    return {
      enabled: c.enabled !== false,
      notifyDone: c.notifyDone !== false,
      onlyWhenUnwatched: c.onlyWhenUnwatched !== false,
      sound: c.sound === true,
    };
  }

  /** True when the user is actively looking at this exact session right now. */
  isWatching(sessionId: string): boolean {
    const win = this.mainWindow;
    const focused = !!win && !win.isDestroyed() && win.isFocused();
    return focused && this.activeSessionId === sessionId;
  }

  /**
   * Called after a hook event updates `ambientState`. Fires at most one
   * OS notification per meaningful transition and mirrors it into the in-app
   * notification center (silently — no toast — when the user is already
   * watching that agent).
   */
  notifyOnTransition(session: ClaudeSessionState, prevState: SessionAmbientState): void {
    const cfg = this.cfg();
    const next = session.ambientState;
    if (next === prevState) return;

    const needsYou = NEEDS_YOU.includes(next) && !NEEDS_YOU.includes(prevState);
    const done = cfg.notifyDone && next === 'idle' && WORKING.includes(prevState);
    if (!needsYou && !done) return;

    const label = agentLabel(session);
    let title: string;
    let body: string;
    if (needsYou && next === 'waiting_approval') {
      title = `${label} needs approval`;
      const tool = session.pendingApproval?.toolName;
      const gist = summarizeToolInput(session.pendingApproval?.toolInput);
      body = tool
        ? truncate(`Allow ${tool}${gist ? ` — ${gist}` : ''}?`, 180)
        : 'A tool call is waiting for your approval.';
    } else if (needsYou) {
      title = `${label} is waiting for input`;
      const questions = session.pendingQuestions ?? [];
      const first = questions[0]?.question;
      const more = questions.length > 1 ? ` (+${questions.length - 1} more)` : '';
      body = first ? truncate(first, 180) + more : 'The agent asked you a question.';
    } else {
      title = `${label} finished`;
      const cost = sessionCost(session);
      body =
        'Ready for your next step.' + (cost > 0 ? ` Spent $${cost.toFixed(2)} this session.` : '');
    }

    const watching = this.isWatching(session.sessionId);

    // In-app mirror: always recorded so the center is a complete history, but
    // toast-silent when the event is already on screen in front of the user.
    this.postInApp({
      level: needsYou ? 'warn' : 'success',
      title,
      body,
      source: 'agent',
      sessionId: session.sessionId,
      // One live slot per session+kind: a newer approval replaces the older
      // center entry rather than stacking a backlog of stale prompts.
      key: `agent:${session.sessionId}:${needsYou ? 'needs-you' : 'done'}`,
      silent: watching,
    });

    if (!cfg.enabled) return;
    if (cfg.onlyWhenUnwatched && watching) return;

    if (Notification.isSupported()) {
      this.showOsNotification(title, body, () => this.focusAgent(session.sessionId));
    }

    // Flash the taskbar for needs-you events (the Windows "attention" signal),
    // but only when the window isn't already focused.
    const win = this.mainWindow;
    if (needsYou && win && !win.isDestroyed() && !win.isFocused()) {
      try {
        win.flashFrame(true);
      } catch {
        /* noop */
      }
    }
  }

  /** Bring the window forward and ask the renderer to select this agent. */
  focusAgent(sessionId: string): void {
    const win = this.focusWindow();
    win?.webContents.send(IPC.NOTIFY_FOCUS_AGENT, sessionId);
  }

  /** Bring the window forward (notification clicks with no agent target). */
  focusWindow(): BrowserWindow | null {
    const win = this.mainWindow;
    if (!win || win.isDestroyed()) return null;
    try {
      if (win.isMinimized()) win.restore();
      win.show();
      if (process.platform === 'win32' && !win.isFocused()) {
        // Windows won't let a background process steal foreground — plain
        // show()+focus() from a notification click often just flashes the
        // taskbar. Briefly pinning the window on top actually raises it.
        win.setAlwaysOnTop(true);
        win.focus();
        win.setAlwaysOnTop(false);
      } else {
        win.focus();
      }
      win.flashFrame(false);
    } catch {
      /* noop */
    }
    return win;
  }

  /**
   * Escalate a renderer-ingested notification (hub `notify.post` event or a
   * renderer-internal post) to an OS notification. The renderer calls this only
   * when the window is unfocused — main-originated notifications never route
   * here (they make their own OS decision at the source), so nothing fires
   * twice. Clicking hands the notification back to the renderer's activate path
   * (NOTIFY_ACTIVATE) so the center marks it read and navigates to its target.
   */
  escalateFromRenderer(n: InAppNotification): void {
    if (!n || typeof n.title !== 'string' || !n.title) return;
    if (n.silent) return; // silent means silent on every surface
    const cfg = this.cfg();
    if (!cfg.enabled || !Notification.isSupported()) return;

    this.showOsNotification(n.title, n.body ?? '', () => {
      const win = this.focusWindow();
      win?.webContents.send(IPC.NOTIFY_ACTIVATE, n);
    });
  }

  /** Create + show an OS notification, guarded against GC until it closes. */
  private showOsNotification(title: string, body: string, onClick: () => void): void {
    // 'close' isn't guaranteed on every platform (Windows toasts that slide
    // into the Action Center may never report it), so bound the held set —
    // a stale reference is harmless, an unbounded set is a leak.
    if (this.liveToasts.size > 50) this.liveToasts.clear();
    const notification = new Notification({
      title,
      body,
      silent: !this.cfg().sound,
      icon: appIconPath() ?? undefined,
    });
    this.liveToasts.add(notification);
    const release = () => this.liveToasts.delete(notification);
    notification.on('click', () => {
      release();
      onClick();
    });
    notification.on('close', release);
    // macOS (Electron 42+, UNUserNotificationCenter): unsigned dev builds get
    // no OS notification and emit 'failed' instead ('failed' is macOS-only;
    // Windows drops are invisible to us). The in-app center already recorded
    // the event, so just make the drop visible in logs.
    notification.on('failed', (_e, err) => {
      release();
      console.warn(`[notify] OS notification failed (in-app center still has it): ${err}`);
    });
    notification.show();
  }

  /** Deliver a notification to the renderer's notification center. Buffered
   *  until the renderer finishes loading so early raises aren't lost. */
  postInApp(n: Omit<InAppNotification, 'id' | 'createdAt'> & { id?: string }): void {
    const full: InAppNotification = {
      ...n,
      id: n.id ?? randomUUID(),
      createdAt: Date.now(),
    };
    this.pendingInApp.push(full);
    this.flushInApp();
  }

  private flushInApp(): void {
    const win = this.mainWindow;
    if (!this.rendererReady || !win || win.isDestroyed()) return;
    for (const n of this.pendingInApp.splice(0)) {
      win.webContents.send(IPC.NOTIFY_IN_APP, n);
    }
  }
}

export const agentNotifier = new AgentNotifier();
