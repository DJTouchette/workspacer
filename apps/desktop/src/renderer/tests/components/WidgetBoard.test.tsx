import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import React from 'react';
import { WidgetBoard } from '../../src/components/widgets/WidgetBoard';
import { ConfigContext, type ConfigContextValue } from '../../src/contexts/ConfigContext';
import { DEFAULT_CONFIG } from '../../src/hooks/configDefaults';
import type { Config } from '../../src/hooks/useConfig';
import type { PluginWidget } from '../../src/types/plugin';
import type { WidgetPlacement } from '../../src/types/widget';
import { WIDGET_SPANS } from '../../src/types/widget';

/**
 * The project widget board: placement persistence keyed by cwd, the closed size
 * vocabulary, and the fallbacks that keep a stale placement renderable.
 *
 * git status is stubbed — the point here is the board, not the widget bodies.
 */

vi.mock('../../src/lib/gitQueries', async () => {
  const actual = await vi.importActual<Record<string, unknown>>('../../src/lib/gitQueries');
  return {
    ...actual,
    GitClient: class {
      async status() {
        return { branch: 'master', files: [{ path: 'a.ts', staged: 'M', unstaged: ' ' }] };
      }
    },
  };
});

const CWD = 'C:\\Users\\me\\work\\repo';
/** The same cwd after projectKey normalization — what must land in config. */
const KEY = 'C:/Users/me/work/repo';

function renderBoard(opts: { placements?: WidgetPlacement[]; available?: PluginWidget[] } = {}) {
  const save = vi.fn().mockResolvedValue(DEFAULT_CONFIG as unknown as Config);
  const config = {
    ...DEFAULT_CONFIG,
    widgets: opts.placements ? { [KEY]: opts.placements } : {},
  } as unknown as Config;
  const value: ConfigContextValue = { config, loaded: true, reload: vi.fn(), save };
  const ui = render(
    <ConfigContext.Provider value={value}>
      <WidgetBoard cwd={CWD} snapshot={null} available={opts.available ?? []} />
    </ConfigContext.Provider>,
  );
  return { save, ...ui };
}

beforeEach(() => vi.clearAllMocks());

describe('WidgetBoard placement', () => {
  it('persists a picked widget under the normalized cwd key', async () => {
    const { save } = renderBoard();
    fireEvent.click(screen.getByTitle('Add a widget'));
    fireEvent.click(screen.getByText('Git'));

    expect(save).toHaveBeenCalledTimes(1);
    // Backslashes normalized, so a repo's board and its `scripts` share a key.
    expect(save.mock.calls[0][0]).toEqual({
      widgets: { [KEY]: [{ widget: 'git', size: 'small' }] },
    });
  });

  it('adds a plugin widget namespaced by its plugin id', () => {
    const { save } = renderBoard({
      available: [
        {
          pluginId: 'djtouchette.shiplight',
          pluginName: 'Shiplight',
          id: 'lamp',
          title: 'Ship status',
          url: 'http://127.0.0.1:9211/widget/lamp',
          sizes: ['medium', 'large'],
        },
      ],
    });
    fireEvent.click(screen.getByTitle('Add a widget'));
    fireEvent.click(screen.getByText('Ship status'));

    expect(save.mock.calls[0][0].widgets[KEY]).toEqual([
      // Defaults to the widget's smallest DECLARED size, not to 'small'.
      { plugin: 'djtouchette.shiplight', widget: 'lamp', size: 'medium' },
    ]);
  });

  it('will not place the same widget twice', () => {
    const { save } = renderBoard({ placements: [{ widget: 'git', size: 'small' }] });
    fireEvent.click(screen.getByTitle('Add a widget'));
    // The picker marks an already-placed widget rather than offering it again.
    const entry = screen.getByText(/on board/).closest('button')!;
    expect(entry).toBeDisabled();
    fireEvent.click(entry);
    expect(save).not.toHaveBeenCalled();
  });

  it('removes a widget, persisting the empty board rather than dropping the key', () => {
    const { save } = renderBoard({ placements: [{ widget: 'git', size: 'small' }] });
    fireEvent.click(screen.getByText('Edit'));
    fireEvent.click(screen.getByText('Remove'));
    expect(save.mock.calls[0][0]).toEqual({ widgets: { [KEY]: [] } });
  });

  it('resizes to a class the widget declared', () => {
    const { save } = renderBoard({ placements: [{ widget: 'git', size: 'small' }] });
    fireEvent.click(screen.getByText('Edit'));
    fireEvent.click(
      screen.getByTitle(`large (${WIDGET_SPANS.large.cols}×${WIDGET_SPANS.large.rows})`),
    );
    expect(save.mock.calls[0][0].widgets[KEY]).toEqual([{ widget: 'git', size: 'large' }]);
  });
});

describe('WidgetBoard resilience', () => {
  // A placement outlives the widget it names: plugins get uninstalled and
  // disabled, and config.yaml is hand-editable. Neither may blank the board.
  it('explains a placement whose plugin is gone instead of rendering an empty tile', () => {
    renderBoard({
      placements: [{ plugin: 'gone.plugin', widget: 'lamp', size: 'small' }],
      available: [],
    });
    expect(screen.getByText('Plugin unavailable')).toBeTruthy();
  });

  // The tile that must NAME ITSELF: nothing is drawing the widget, so the
  // placement's own name is the only thing left to identify it by. A board with
  // two disabled plugins otherwise degrades to two identical unlabelled tiles.
  it('names the widget that vanished, not just "unavailable"', () => {
    renderBoard({
      placements: [
        { plugin: 'gone.plugin', widget: 'lamp', size: 'small' },
        { plugin: 'other.plugin', widget: 'radar', size: 'small' },
      ],
      available: [],
    });
    expect(screen.getAllByText('Plugin unavailable')).toHaveLength(2);
    expect(screen.getByText('lamp')).toBeTruthy();
    expect(screen.getByText('radar')).toBeTruthy();
  });

  it('explains an unknown host widget', () => {
    renderBoard({ placements: [{ widget: 'no-such-widget', size: 'small' }] });
    expect(screen.getByText('Unknown widget')).toBeTruthy();
    expect(screen.getByText('no-such-widget')).toBeTruthy();
  });

  // A tile is not a window: the host draws no title over a widget, because the
  // widget's own content is what identifies it and a header ate a fifth of a
  // 148px square to restate it. The name has to survive somewhere though, so
  // both halves are pinned — gone while reading, present while editing.
  it('draws no title over a widget, and names it in edit mode', () => {
    renderBoard({
      placements: [{ plugin: 'djtouchette.shiplight', widget: 'lamp', size: 'small' }],
      available: [
        {
          pluginId: 'djtouchette.shiplight',
          pluginName: 'Shiplight',
          id: 'lamp',
          title: 'Ship status',
          url: 'http://127.0.0.1:9211/widget/lamp',
          sizes: ['small'],
        },
      ],
    });
    expect(screen.queryByText('Ship status')).toBeNull();
    fireEvent.click(screen.getByText('Edit'));
    expect(screen.getByText('Ship status')).toBeTruthy();
  });

  it('clamps a size the widget no longer supports down to one it does', () => {
    // 'usage' declares small+medium only; a stale board asking for large must
    // render at medium rather than spanning two rows it never designed for.
    const { container } = renderBoard({ placements: [{ widget: 'usage', size: 'large' }] });
    const tile = container.querySelector('[style*="grid-column"]') as HTMLElement;
    expect(tile.style.gridColumn).toBe(`span ${WIDGET_SPANS.medium.cols}`);
    expect(tile.style.gridRow).toBe(`span ${WIDGET_SPANS.medium.rows}`);
  });
});
