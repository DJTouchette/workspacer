/** Trusted chrome actions, pinned to the live owning chat. */
import type { HtmlCardAction } from '../../../../main/shared/htmlCard';
import type { HtmlCardDiffResult } from '../../../../main/shared/htmlCardDiff';
import { requestSessionWatch } from '../watchBus';
import { dispatchInsert } from '../libraryBus';
import { postNotification } from '../notificationBus';

export interface HtmlCardHost {
  sessionId?: string;
  paneId?: string;
  cwd?: string;
  /** Pane checks its current identity when async lookups finish. */
  isCurrent?: () => boolean;
}
export type CardActionOutcome = { ok: true } | { ok: false; message: string };
function refuse(message: string): CardActionOutcome {
  postNotification({
    title: 'Card action unavailable',
    body: message,
    level: 'warn',
    source: 'chat',
  });
  return { ok: false, message };
}
export async function performCardAction(
  action: HtmlCardAction,
  host: HtmlCardHost,
  showDiff?: (diff: Extract<HtmlCardDiffResult, { ok: true }>) => void,
): Promise<CardActionOutcome> {
  try {
    if (!host.sessionId || host.isCurrent?.() === false)
      return refuse('This card has no live owning chat.');
    const get = window.electronAPI?.getClaudeSession;
    const owner = get ? await get(host.sessionId) : null;
    if (
      !owner ||
      owner.status === 'ended' ||
      owner.hubOffline ||
      owner.hub ||
      owner.sessionId !== host.sessionId ||
      (owner.liveCwd || owner.cwd) !== host.cwd ||
      host.isCurrent?.() === false
    )
      return refuse('The owning chat is no longer available.');
    switch (action.kind) {
      case 'open_worker': {
        const target = await get!(action.sessionId);
        if (!target || target.sessionId !== action.sessionId || target.status === 'ended')
          return refuse('That agent is no longer running.');
        if (
          target.parentSessionId !== owner.sessionId ||
          target.hub ||
          target.hubOffline ||
          host.isCurrent?.() === false
        )
          return refuse('That agent is not a live worker of this chat.');
        const currentOwner = await get!(host.sessionId);
        if (
          !currentOwner ||
          currentOwner.status === 'ended' ||
          currentOwner.hub ||
          currentOwner.sessionId !== host.sessionId ||
          (currentOwner.liveCwd || currentOwner.cwd) !== host.cwd ||
          host.isCurrent?.() === false
        )
          return refuse('The owning chat changed.');
        requestSessionWatch({
          sessionId: target.sessionId,
          cwd: target.liveCwd || target.cwd,
          title: target.label || 'Worker',
        });
        return { ok: true };
      }
      case 'view_diff': {
        const read = window.electronAPI?.htmlCardReadDiff;
        if (!read || !showDiff) return refuse('Card diffs are not available on this client.');
        const diff = await read(action.path, owner.sessionId);
        if (!diff.ok) return refuse(diff.error);
        if (host.isCurrent?.() === false) return refuse('The owning chat changed.');
        showDiff(diff);
        return { ok: true };
      }
      case 'fill_composer':
        if (!host.paneId) return refuse('This card has no owning composer.');
        dispatchInsert(action.text, { sessionId: owner.sessionId, paneId: host.paneId });
        return { ok: true };
      default:
        return refuse('Unsupported card action.');
    }
  } catch {
    return refuse('The card target could not be checked.');
  }
}
export function actionAvailable(action: HtmlCardAction, host: HtmlCardHost | null): boolean {
  if (!host?.sessionId) return false;
  return action.kind === 'fill_composer' ? Boolean(host.paneId) : true;
}
