import React, { useMemo, useRef, useState } from 'react';
import { Plus, X, Check } from 'lucide-react';
import type { ClaudeSessionSnapshot } from '../../types/claudeSession';
import type { PluginWidget } from '../../types/plugin';
import type { WidgetPlacement, WidgetSize } from '../../types/widget';
import {
  WIDGET_CELL,
  WIDGET_COLUMNS,
  WIDGET_GAP,
  WIDGET_PAD,
  WIDGET_SIZES,
  WIDGET_SPANS,
  clampWidgetSize,
  widgetKey,
} from '../../types/widget';
import { claudeColors as colors } from '../claude-shared';
import { useWidgetBoard } from '../../hooks/useWidgetBoard';
import { usePluginWebview } from '../../hooks/usePluginWebview';
import { HOST_WIDGETS, hostWidget } from './hostWidgets';

/**
 * A project's widget board: the iPhone-style grid in the inspector rail.
 *
 * Two columns, three closed size classes (see types/widget.ts). Rows are locked
 * to WIDGET_CELL so a medium is always exactly as tall as the small beside it;
 * columns are fluid so the board survives a rail that gains a resize handle.
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
        <span style={{ fontSize: 10, color: colors.mutedDim, letterSpacing: 0.4 }}>
          {projectLabel(cwd).toUpperCase()}
        </span>
        <div style={{ marginLeft: 'auto', display: 'flex', gap: 2 }}>
          {board.placements.length > 0 && (
            <ChromeButton
              title={editing ? 'Done' : 'Edit board'}
              onClick={() => setEditing((v) => !v)}
              active={editing}
            >
              {editing ? <Check size={13} strokeWidth={2} /> : <span style={{ fontSize: 11 }}>Edit</span>}
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

/** One placed widget: chrome plus either an inline host view or a plugin guest. */
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

  return (
    <div
      style={{
        gridColumn: `span ${span.cols}`,
        gridRow: `span ${span.rows}`,
        position: 'relative',
        display: 'flex',
        flexDirection: 'column',
        borderRadius: 14,
        border: `1px solid ${colors.border}`,
        background: 'rgba(255,255,255,0.022)',
        overflow: 'hidden',
        minWidth: 0,
      }}
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 5,
          padding: '7px 9px 3px',
          color: colors.mutedDim,
          flexShrink: 0,
        }}
      >
        <span style={{ display: 'flex', flexShrink: 0 }}>{host?.icon ?? plugin?.icon ?? '🔌'}</span>
        <span
          style={{
            fontSize: 10,
            letterSpacing: 0.3,
            textTransform: 'uppercase',
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
          }}
        >
          {host?.title ?? plugin?.title ?? placement.widget}
        </span>
      </div>

      <div style={{ flex: 1, minHeight: 0, padding: '0 9px 9px', display: 'flex' }}>
        {missing ? (
          // The plugin was uninstalled or disabled but its placement remains.
          // Say so rather than rendering an empty tile — and leave the placement
          // alone, so re-enabling the plugin brings the widget back.
          <div style={{ fontSize: 11, color: colors.mutedDim, margin: 'auto', textAlign: 'center' }}>
            {placement.plugin ? 'Plugin unavailable' : 'Unknown widget'}
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
            background: 'rgba(0,0,0,0.62)',
            display: 'flex',
            flexDirection: 'column',
            alignItems: 'center',
            justifyContent: 'center',
            gap: 8,
          }}
        >
          <div style={{ display: 'flex', gap: 4 }}>
            {WIDGET_SIZES.filter((s) => supported.includes(s)).map((s) => (
              <button
                key={s}
                onClick={() => onResize(s)}
                title={`${s} (${WIDGET_SPANS[s].cols}×${WIDGET_SPANS[s].rows})`}
                style={{
                  border: `1px solid ${s === size ? colors.text : colors.border}`,
                  background: s === size ? 'rgba(255,255,255,0.10)' : 'transparent',
                  color: s === size ? colors.text : colors.mutedDim,
                  borderRadius: 6,
                  fontSize: 10,
                  padding: '3px 7px',
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
              color: '#e5534b',
              borderRadius: 6,
              fontSize: 11,
              padding: '3px 9px',
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
    </div>
  );
};

/**
 * A plugin widget's guest.
 *
 * Same `<webview>` mechanism and same `persist:browser` partition as a plugin
 * pane, so a plugin's pane and widgets are same-origin and Chromium can reuse
 * one renderer process across them. Measured marginal cost of a guest is ~30MB
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
    <webview
      // Remount on url change rather than mutating src — a guest that has
      // already navigated ignores a changed src attribute.
      key={src}
      ref={ref as never}
      src={src}
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
