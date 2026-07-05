/**
 * OS notifications + taskbar attention for agent state changes.
 *
 * Driven from `claudeSessionStore` on every ambient-state transition. We fire
 * an OS notification when an agent transitions *into* a needs-you state
 * (approval / input) or *finishes* (working → idle), so you can babysit agents
 * you're not actively watching. Clicking a notification focuses the window and
 * tells the renderer to jump to that agent.
 *
 * Behaviour is config-driven (`notifications` block in config.yaml); the
 * defaults match the product choice: needs-you + done, only when unwatched,
 * silent.
 */

import { BrowserWindow, Notification } from 'electron';
import { configService } from './configService';
import { appIconPath } from '../lib/appIcon';
import type { ClaudeSessionState, SessionAmbientState } from './claudeSessionStore';

const NEEDS_YOU: SessionAmbientState[] = ['waiting_approval', 'waiting_input'];
const WORKING: SessionAmbientState[] = ['streaming', 'thinking'];

interface NotificationsConfig {
  enabled: boolean;
  /** Notify when an agent finishes (working → idle). */
  notifyDone: boolean;
  /** Suppress notifications for the agent you're currently looking at. */
  onlyWhenUnwatched: boolean;
  sound: boolean;
}

/** Human label for a session — the basename of its working directory. */
function agentLabel(cwd: string): string {
  if (!cwd) return 'Agent';
  const parts = cwd.replace(/[/\\]+$/, '').split(/[/\\]/);
  return parts[parts.length - 1] || cwd;
}

class AgentNotifier {
  private mainWindow: BrowserWindow | null = null;
  /** sessionId the renderer currently has on screen (null if none/stopped). */
  private activeSessionId: string | null = null;

  setMainWindow(win: BrowserWindow): void {
    this.mainWindow = win;
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
  private isWatching(sessionId: string): boolean {
    const win = this.mainWindow;
    const focused = !!win && !win.isDestroyed() && win.isFocused();
    return focused && this.activeSessionId === sessionId;
  }

  /**
   * Called after a hook event updates `ambientState`. Fires at most one
   * notification per meaningful transition.
   */
  notifyOnTransition(session: ClaudeSessionState, prevState: SessionAmbientState): void {
    const cfg = this.cfg();
    if (!cfg.enabled) return;

    const next = session.ambientState;
    if (next === prevState) return;

    const needsYou = NEEDS_YOU.includes(next) && !NEEDS_YOU.includes(prevState);
    const done = cfg.notifyDone && next === 'idle' && WORKING.includes(prevState);
    if (!needsYou && !done) return;

    if (cfg.onlyWhenUnwatched && this.isWatching(session.sessionId)) return;

    const label = agentLabel(session.cwd);
    const title = needsYou
      ? next === 'waiting_approval'
        ? `${label} needs approval`
        : `${label} is waiting for input`
      : `${label} finished`;
    const body = needsYou
      ? 'Click to jump to the agent.'
      : 'The agent is idle and ready for your next step.';

    if (Notification.isSupported()) {
      const icon = appIconPath() ?? undefined;
      const notification = new Notification({ title, body, silent: !cfg.sound, icon });
      notification.on('click', () => this.focusAgent(session.sessionId));
      notification.show();
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
  private focusAgent(sessionId: string): void {
    const win = this.mainWindow;
    if (!win || win.isDestroyed()) return;
    try {
      if (win.isMinimized()) win.restore();
      win.show();
      win.focus();
      win.flashFrame(false);
    } catch {
      /* noop */
    }
    win.webContents.send('notify:focus-agent', sessionId);
  }
}

export const agentNotifier = new AgentNotifier();
