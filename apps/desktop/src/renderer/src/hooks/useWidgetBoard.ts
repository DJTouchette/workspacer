import { useCallback, useMemo } from 'react';
import { useConfig } from './useConfig';
import { resolveProjectKey } from '../lib/projectKey';
import type { WidgetPlacement, WidgetSize } from '../types/widget';
import { widgetKey } from '../types/widget';

/**
 * Read/write one project's widget board.
 *
 * The board is keyed by normalized cwd in config.yaml, exactly as `scripts` is —
 * NOT in the hub layout doc, which is per-AgentWorkspace and broadcast to every
 * connected client. Several agents can share a cwd and they all see one board.
 *
 * Every mutation writes the whole array for this key. `save` sends a minimal
 * patch (see lib/configPatch) and config's deepMerge replaces arrays wholesale,
 * so removing the last widget from a project persists as [] rather than merging
 * the old contents back.
 */
export interface WidgetBoard {
  /** Placements for this project, in board order. Empty when there's no cwd. */
  placements: WidgetPlacement[];
  /** True once config has loaded — until then `placements` is not yet meaningful. */
  ready: boolean;
  add: (placement: WidgetPlacement) => void;
  remove: (placement: Pick<WidgetPlacement, 'plugin' | 'widget'>) => void;
  resize: (placement: Pick<WidgetPlacement, 'plugin' | 'widget'>, size: WidgetSize) => void;
  /** Move a widget to a new index, for reordering. */
  move: (from: number, to: number) => void;
  /** Whether this widget is already on the board (the picker greys it out). */
  has: (placement: Pick<WidgetPlacement, 'plugin' | 'widget'>) => boolean;
}

export function useWidgetBoard(cwd: string | undefined): WidgetBoard {
  const { config, loaded, save } = useConfig();
  // Resolved against the existing map so a case-only variant of an already-saved
  // path reuses that board instead of starting an empty second one.
  const key = cwd ? resolveProjectKey(config.widgets, cwd) : '';

  const placements = useMemo<WidgetPlacement[]>(
    () => (key ? (config.widgets?.[key] ?? []) : []),
    [config.widgets, key],
  );

  const write = useCallback(
    (next: WidgetPlacement[]) => {
      if (!key) return;
      save({ widgets: { ...(config.widgets ?? {}), [key]: next } });
    },
    [config.widgets, key, save],
  );

  const has = useCallback(
    (p: Pick<WidgetPlacement, 'plugin' | 'widget'>) =>
      placements.some((x) => widgetKey(x) === widgetKey(p)),
    [placements],
  );

  const add = useCallback(
    (p: WidgetPlacement) => {
      // A widget appears at most once per board: two copies of the same
      // glanceable view is never what someone meant, and it would make the
      // placement key ambiguous.
      if (has(p)) return;
      write([...placements, p]);
    },
    [has, placements, write],
  );

  const remove = useCallback(
    (p: Pick<WidgetPlacement, 'plugin' | 'widget'>) =>
      write(placements.filter((x) => widgetKey(x) !== widgetKey(p))),
    [placements, write],
  );

  const resize = useCallback(
    (p: Pick<WidgetPlacement, 'plugin' | 'widget'>, size: WidgetSize) =>
      write(placements.map((x) => (widgetKey(x) === widgetKey(p) ? { ...x, size } : x))),
    [placements, write],
  );

  const move = useCallback(
    (from: number, to: number) => {
      if (from === to || from < 0 || to < 0) return;
      if (from >= placements.length || to >= placements.length) return;
      const next = [...placements];
      const [moved] = next.splice(from, 1);
      next.splice(to, 0, moved);
      write(next);
    },
    [placements, write],
  );

  return { placements, ready: loaded, add, remove, resize, move, has };
}
