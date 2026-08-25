import React, { useMemo, useRef, useState } from 'react';
import { Plus, X, Check } from 'lucide-react';
import type { ClaudeSessionSnapshot } from '../../types/claudeSession';
import type { PluginWidget } from '../../types/plugin';
import type { WidgetPlacement, WidgetSize } from '../../types/widget';
import {
  WIDGET_CELL,
  WIDGET_COLUMNS,
  WIDGET_GAP,
  WIDGET_INSET,
  WIDGET_PAD,
  WIDGET_SIZES,
  WIDGET_SPANS,
  clampWidgetSize,
  widgetKey,
} from '../../types/widget';
import { claudeColors as colors } from '../claude-shared';
import { Surface } from '../Surface';
import { useWidgetBoard } from '../../hooks/useWidgetBoard';
import { usePluginWebview } from '../../hooks/usePluginWebview';
import GuestFrame from '../GuestFrame';
import { HOST_WIDGETS, hostWidget } from './hostWidgets';

/**
 * A project's widget board: the iPhone-style grid in the inspector rail.
 *
 * Two columns, three closed size classes (see types/widget.ts). Rows are locked
 * to WIDGET_CELL so a medium is always exactly as tall as the small beside it;
 * columns are fluid so the board survives a rail that gains a resize handle.
 *
 * Tiles carry no title bar — see {@link WidgetCell} for why, and for where a
 * widget's name went instead.
 *
 * Scope: the board belongs to the *cwd*, not the session. It renders as a
 * sibling of InspectorCard rather than a sixth tab inside it, because that card
 * guarantees it draws purely from the snapshot it's handed and never fetches —
 * a project-scoped, self-fetching grid does not belong under that invariant.
 *
 * Lifecycle: everything here unmounts when the rail closes, which is what makes
 * plugin widgets affordable. Their guests are torn down by React rather than by
 * the hibernation sweep, which only walks panes inside tabs and never sees these
 * (see lib/hibernation.ts).
 */
export const WidgetBoard: React.FC<{
  cwd: string;
  snapshot: ClaudeSessionSnapshot | null;
  /** Plugin-contributed widgets available to place (from usePlugins). */
  available: PluginWidget[];
}> = ({ cwd, snapshot, available }) => {
  const board = useWidgetBoard(cwd);
  const [editing, setEditing] = useState(false);
  const [picking, setPicking] = useState(false);

  const byKey = useMemo(() => {
    const m = new Map<string, PluginWidget>();
    for (const w of available) m.set(`${w.pluginId}:${w.id}`, w);
    return m;
  }, [available]);

  if (!cwd) {
    return <BoardMessage>Open an agent in a project to pin widgets.</BoardMessage>;
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', minHeight: 0, flex: 1 }}>
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 6,
          padding: `6px ${WIDGET_PAD}px`,
          flexShrink: 0,
        }}
      >
        {/* The board's one label. It names the DIRECTORY, which is the thing a
            tile can't say for itself and the thing the board is scoped to. */}
        <span
          style={{
            fontSize: '0.6rem',
            color: colors.mutedDim,
            letterSpacing: 0.4,
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
          }}
        >
          {projectLabel(cwd).toUpperCase()}
        </span>
        <div style={{ marginLeft: 'auto', display: 'flex', gap: 2 }}>
          {board.placements.length > 0 && (
            <ChromeButton
              title={editing ? 'Done' : 'Edit board'}
              onClick={() => setEditing((v) => !v)}
              active={editing}
            >
              {editing ? (
                <Check size={13} strokeWidth={2} />
              ) : (
                <span style={{ fontSize: '0.66rem' }}>Edit</span>
              )}
            </ChromeButton>
          )}
          <ChromeButton title="Add a widget" onClick={() => setPicking((v) => !v)} active={picking}>
            <Plus size={13} strokeWidth={2} />
          </ChromeButton>
        </div>
      </div>

      {picking && (
        <WidgetPicker
          available={available}
          has={board.has}
          onPick={(p) => {
            board.add(p);
            setPicking(false);
          }}
        />
      )}

      <div style={{ overflowY: 'auto', overflowX: 'hidden', flex: 1, minHeight: 0 }}>
        {board.placements.length === 0 && !picking ? (
          <BoardMessage>
            No widgets yet. <b>+</b> pins one to this project.
          </BoardMessage>
        ) : (
          <div
            style={{
              display: 'grid',
              gridTemplateColumns: `repeat(${WIDGET_COLUMNS}, minmax(0, 1fr))`,
              gridAutoRows: `${WIDGET_CELL}px`,
              gap: WIDGET_GAP,
              padding: WIDGET_PAD,
              paddingTop: 0,
            }}
          >
            {board.placements.map((p) => (
              <WidgetCell
                key={widgetKey(p)}
                placement={p}
                cwd={cwd}
                snapshot={snapshot}
                plugin={p.plugin ? byKey.get(`${p.plugin}:${p.widget}`) : undefined}
                editing={editing}
                onRemove={() => board.remove(p)}
                onResize={(s) => board.resize(p, s)}
              />
            ))}
          </div>
        )}
      </div>
    </div>
  );
};

// ---------------------------------------------------------------------------

/**
 * One placed widget: a tile, and either an inline host view or a plugin guest
 * filling it edge to edge.
 *
 * NO TITLE BAR, deliberately. An iPhone widget is not a window: it has no
 * chrome, and its identity comes from what it draws — the lamp IS the ship
 * status, the branch name IS git. A host-drawn "SHIP STATUS" header above a
 * widget whose whole job is to be read in one glance spent a fifth of a 148px
 * tile restating what the tile already said, and pushed the content it was
 * labelling into the top-left corner with dead space under it.
 *
 * The name doesn't disappear — it moves to where it's actually needed: the
 * picker names every widget, edit mode labels each tile (that is when you're
 * identifying rather than reading them), and a placement whose widget is gone
 * names it in the tile, since there is nothing else left to identify it by.
 */
const WidgetCell: React.FC<{
  placement: WidgetPlacement;
  cwd: string;
  snapshot: ClaudeSessionSnapshot | null;
  plugin: PluginWidget | undefined;
  editing: boolean;
  onRemove: () => void;
  onResize: (s: WidgetSize) => void;
}> = ({ placement, cwd, snapshot, plugin, editing, onRemove, onResize }) => {
  const host = placement.plugin ? undefined : hostWidget(placement.widget);
  const supported = host?.sizes ?? plugin?.sizes ?? WIDGET_SIZES;
  // A placement outlives the widget it names: a plugin update can drop a size
  // class, and config.yaml is hand-editable. Render at something the widget
  // actually declared rather than at whatever was asked for.
  const size = clampWidgetSize(placement.size, supported);
  const span = WIDGET_SPANS[size];

  const missing = !host && !plugin;
  const title = host?.title ?? plugin?.title ?? placement.widget;

  return (
    // raised: fill + lighting, no border. The tile sits directly on the rail,
    // so it is an outermost surface — the old border+fill was the exact pair
    // Surface exists to prevent (see DESIGN_LANGUAGE §5).
    <Surface
      elevation="raised"
      radius="lg"
      style={{
        gridColumn: `span ${span.cols}`,
        gridRow: `span ${span.rows}`,
        position: 'relative',
        display: 'flex',
        padding: WIDGET_INSET,
        overflow: 'hidden',
        minWidth: 0,
      }}
    >
      <div style={{ flex: 1, minWidth: 0, minHeight: 0, display: 'flex' }}>
        {missing ? (
          // The plugin was uninstalled or disabled but its placement remains.
          // Say so rather than rendering an empty tile — and leave the placement
          // alone, so re-enabling the plugin brings the widget back. This is the
          // one case that must name itself: nothing is drawing the widget.
          <div
            style={{
              margin: 'auto',
              textAlign: 'center',
              fontSize: '0.66rem',
              color: colors.mutedDim,
              minWidth: 0,
            }}
          >
            <div style={{ color: colors.muted }}>{title}</div>
            <div>{placement.plugin ? 'Plugin unavailable' : 'Unknown widget'}</div>
          </div>
        ) : host ? (
          <host.Render cwd={cwd} size={size} snapshot={snapshot} />
        ) : (
          <PluginWidgetView widget={plugin!} cwd={cwd} />
        )}
      </div>

      {editing && (
        <div
          style={{
            position: 'absolute',
            inset: 0,
            background: 'var(--wks-overlay)',
            display: 'flex',
            flexDirection: 'column',
            alignItems: 'center',
            justifyContent: 'center',
            gap: 8,
            padding: WIDGET_INSET,
          }}
        >
          {/* Edit mode is when a tile needs a name: you're picking one out to
              resize or remove, not reading it. */}
          <div
            style={{
              fontSize: '0.66rem',
              color: colors.text,
              maxWidth: '100%',
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
            }}
          >
            {title}
          </div>
          <div style={{ display: 'flex', gap: 4 }}>
            {WIDGET_SIZES.filter((s) => supported.includes(s)).map((s) => (
              <button
                key={s}
                onClick={() => onResize(s)}
                title={`${s} (${WIDGET_SPANS[s].cols}×${WIDGET_SPANS[s].rows})`}
                style={{
                  border: `1px solid ${s === size ? colors.text : colors.border}`,
                  background: s === size ? 'var(--wks-bg-selected)' : 'transparent',
                  color: s === size ? colors.text : colors.mutedDim,
                  borderRadius: 'var(--wks-radius-sm)',
                  fontSize: '0.6rem',
                  padding: '4px 8px',
                  cursor: 'pointer',
                }}
              >
                {s[0].toUpperCase()}
              </button>
            ))}
          </div>
          <button
            onClick={onRemove}
            style={{
              border: `1px solid ${colors.border}`,
              background: 'transparent',
              color: colors.error,
              borderRadius: 'var(--wks-radius-sm)',
              fontSize: '0.66rem',
              padding: '4px 10px',
              cursor: 'pointer',
              display: 'flex',
              alignItems: 'center',
              gap: 4,
            }}
          >
            <X size={11} strokeWidth={2} /> Remove
          </button>
        </div>
      )}
    </Surface>
  );
};

/**
 * A plugin widget's guest.
 *
 * Same guest mechanism and same `persist:browser` partition as a plugin pane
 * (`<webview>` on the desktop, a sandboxed `<iframe>` on /app — see
 * `lib/guestFrame.ts`), so a plugin's pane and widgets are same-origin and
 * Chromium can reuse one renderer process across them. Measured marginal cost of a guest is ~30MB
 * private — real but affordable at a board-sized count, and reclaimed entirely
 * when the rail closes and this unmounts.
 *
 * The cwd rides in the query so a widget can scope itself to the project without
 * a bus round-trip; the busToken is what the hub keys its capability grant on.
 */
const PluginWidgetView: React.FC<{ widget: PluginWidget; cwd: string }> = ({ widget, cwd }) => {
  const ref = useRef<HTMLElement | null>(null);
  // Same theme-token + settings injection a plugin PANE gets. A widget is the
  // same kind of guest in a smaller frame: without this it would ignore the
  // user's theme (falling back to whatever colours the author hardcoded) and be
  // unable to read its own settings.
  usePluginWebview(ref, widget.pluginId);

  const src = useMemo(() => {
    try {
      const u = new URL(widget.url);
      if (widget.busToken) u.searchParams.set('busToken', widget.busToken);
      u.searchParams.set('cwd', cwd);
      u.searchParams.set('surface', 'widget');
      return u.toString();
    } catch {
      return widget.url;
    }
  }, [widget.url, widget.busToken, cwd]);

  return (
    <GuestFrame
      // Remount on url change rather than mutating src — a guest that has
      // already navigated ignores a changed src attribute.
      key={src}
      ref={ref}
      src={src}
      title={widget.title}
      style={{ flex: 1, width: '100%', height: '100%', border: 'none' }}
      partition="persist:browser"
    />
  );
};

/** The add-widget list: host widgets first, then each plugin's contributions. */
const WidgetPicker: React.FC<{
  available: PluginWidget[];
  has: (p: Pick<WidgetPlacement, 'plugin' | 'widget'>) => boolean;
  onPick: (p: WidgetPlacement) => void;
}> = ({ available, has, onPick }) => {
  const rows: Array<{ key: string; label: string; group: string; placement: WidgetPlacement }> = [
    ...HOST_WIDGETS.map((w) => ({
      key: `host:${w.id}`,
      label: w.title,
      group: 'Built in',
      // Default to the widget's smallest declared footprint — the least
      // presumptuous choice, and resizing up is one click in edit mode.
      placement: { widget: w.id, size: w.sizes[0] } as WidgetPlacement,
    })),
    ...available.map((w) => ({
      key: `${w.pluginId}:${w.id}`,
      label: w.title,
      group: w.pluginName,
      placement: { plugin: w.pluginId, widget: w.id, size: w.sizes[0] } as WidgetPlacement,
    })),
  ];

  let lastGroup = '';
  return (
    <div
      style={{
        maxHeight: 220,
        overflowY: 'auto',
        margin: `0 ${WIDGET_PAD}px 8px`,
        border: `1px solid ${colors.border}`,
        borderRadius: 10,
        background: 'rgba(0,0,0,0.28)',
      }}
    >
      {rows.map((r) => {
        const already = has(r.placement);
        const header = r.group !== lastGroup ? ((lastGroup = r.group), r.group) : null;
        return (
          <React.Fragment key={r.key}>
            {header && (
              <div
                style={{
                  fontSize: 9,
                  color: colors.mutedDim,
                  letterSpacing: 0.5,
                  textTransform: 'uppercase',
                  padding: '6px 9px 2px',
                }}
              >
                {header}
              </div>
            )}
            <button
              disabled={already}
              onClick={() => onPick(r.placement)}
              style={{
                display: 'block',
                width: '100%',
                textAlign: 'left',
                border: 'none',
                background: 'transparent',
                color: already ? colors.mutedDim : colors.text,
                fontSize: 12,
                padding: '5px 9px',
                cursor: already ? 'default' : 'pointer',
                opacity: already ? 0.5 : 1,
              }}
            >
              {r.label}
              {already && <span style={{ fontSize: 10 }}> · on board</span>}
            </button>
          </React.Fragment>
        );
      })}
      {rows.length === 0 && (
        <div style={{ fontSize: 11, color: colors.mutedDim, padding: 9 }}>
          No widgets available.
        </div>
      )}
    </div>
  );
};

const ChromeButton: React.FC<{
  title: string;
  active?: boolean;
  onClick: () => void;
  children: React.ReactNode;
}> = ({ title, active, onClick, children }) => (
  <button
    title={title}
    onClick={onClick}
    style={{
      border: 'none',
      background: active ? 'rgba(255,255,255,0.08)' : 'transparent',
      color: active ? colors.text : colors.mutedDim,
      cursor: 'pointer',
      padding: '2px 6px',
      borderRadius: 6,
      display: 'flex',
      alignItems: 'center',
    }}
  >
    {children}
  </button>
);

const BoardMessage: React.FC<{ children: React.ReactNode }> = ({ children }) => (
  <div
    style={{
      fontSize: 11,
      color: colors.mutedDim,
      padding: `18px ${WIDGET_PAD}px`,
      textAlign: 'center',
      lineHeight: 1.5,
    }}
  >
    {children}
  </div>
);

/** Last path segment of a cwd — the board header's "which project" label. */
function projectLabel(cwd: string): string {
  const parts = cwd.replace(/\\/g, '/').replace(/\/+$/, '').split('/');
  return parts[parts.length - 1] || cwd;
}
