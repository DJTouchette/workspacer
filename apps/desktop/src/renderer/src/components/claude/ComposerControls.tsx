/**
 * Session control pills — "Opus 4.5 ▾ · High ▾ · Full access ▾" — rendered
 * flat (separator-divided, no borders) inside the composer's bottom row in GUI
 * mode, and in the pane's bottom status bar in terminal mode. What each pill
 * can do comes from lib/providerCaps.ts:
 *
 *  - Model:  claude switches live (`/model <id>` submitted through the normal
 *    message path); codex switches live too (claudemon applies
 *    `thread/settings/update` to the running thread — falls back to the
 *    restart confirm when the daemon says it can't, e.g. rollout fallback);
 *    opencode/pi restart with the new model.
 *  - Effort: codex only, restart (`-c model_reasoning_effort=<level>`).
 *  - Permission mode: live where the daemon can drive it (claude via the
 *    verified shift+tab cycle, codex via the adapter's approval flag); when
 *    the daemon reports the switch can't be done live, the pick falls back to
 *    the restart confirm with the daemon's reason. opencode/pi restart.
 *
 * Restart selections go through a confirm step whose copy says whether the
 * conversation survives (claude resumes; codex/opencode start fresh).
 * Displayed values: live telemetry (statusLine/usage model) wins, then the
 * requested-at-spawn `snapshot.settings`, then a placeholder.
 */

import React, { useCallback, useEffect, useRef, useState } from 'react';
import type { ClaudeSessionSnapshot } from '../../types/claudeSession';
import type { AgentProvider } from '../../types/pane';
import { capsFor, permissionModeLabel } from '../../lib/providerCaps';
import { deriveSessionStats } from '../../lib/sessionStats';
import { shortModelLabel } from '../../lib/modelLabel';
import { claudeColors as colors } from '../claude-shared';
import {
  ContextMenu,
  ContextMenuItem,
  ContextMenuLabel,
  ContextMenuSeparator,
} from '../ContextMenu';
import { IconModel } from '../wksIcons';

export interface RestartOverrides {
  model?: string;
  effort?: string;
  permissionMode?: string;
}

interface ModelOption {
  id: string;
  label: string;
  /** Context-window badge ('200K' | '1M'). */
  context?: string;
  /** True for concrete ids observed in sessions (grouped after the aliases). */
  seen?: boolean;
}

/** Context-window chip; the 1M window gets the accent treatment. */
const CtxBadge: React.FC<{ ctx: string }> = ({ ctx }) => {
  const big = ctx === '1M';
  return (
    <span
      style={{
        fontSize: '0.55rem',
        fontWeight: 700,
        padding: '1px 5px',
        borderRadius: 'var(--wks-radius-pill)',
        letterSpacing: '0.04em',
        fontFamily: 'var(--wks-font-mono, monospace)',
        flexShrink: 0,
        color: big ? 'var(--wks-accent-text)' : 'var(--wks-text-faint)',
        border: `1px solid ${
          big ? 'color-mix(in srgb, var(--wks-accent) 45%, transparent)' : 'var(--wks-border-subtle)'
        }`,
        backgroundColor: big ? 'color-mix(in srgb, var(--wks-accent) 12%, transparent)' : 'transparent',
      }}
    >
      {ctx}
    </span>
  );
};

/** Model row: name + context badge (+ ✓ current). */
const modelItemLabel = (m: ModelOption, current: boolean): React.ReactNode => (
  <span style={{ display: 'flex', alignItems: 'center', gap: 7, minWidth: 0 }}>
    <span style={{ fontWeight: 600 }}>{m.label}</span>
    {m.context && <CtxBadge ctx={m.context} />}
    {current && (
      <span style={{ color: 'var(--wks-success)', fontSize: '0.65rem', flexShrink: 0 }}>✓</span>
    )}
  </span>
);

type MenuKind = 'model' | 'effort' | 'permission';

interface MenuState {
  kind: MenuKind;
  x: number;
  y: number;
  /** Set once the user picked a restart-requiring value — confirm view.
   *  `reason` carries the daemon's explanation when a live switch fell back
   *  to the restart path. */
  confirm?: { overrides: RestartOverrides; label: string; reason?: string };
}

const pillStyle: React.CSSProperties = {
  display: 'inline-flex',
  alignItems: 'center',
  gap: 5,
  padding: '3px 8px',
  borderRadius: 'var(--wks-radius-sm)',
  border: 'none',
  background: 'transparent',
  color: colors.muted,
  cursor: 'pointer',
  fontSize: '0.68rem',
  fontFamily: 'inherit',
  fontWeight: 600,
  lineHeight: '16px',
  whiteSpace: 'nowrap',
  maxWidth: 180,
  overflow: 'hidden',
  textOverflow: 'ellipsis',
  flexShrink: 0,
};

/** Thin vertical rule between controls (T3-style separators). */
const Sep: React.FC = () => (
  <span
    aria-hidden
    style={{
      width: 1,
      height: 14,
      flexShrink: 0,
      margin: '0 2px',
      background: colors.borderSubtle,
    }}
  />
);

export const ComposerControls: React.FC<{
  provider: AgentProvider | undefined;
  sessionId: string | null;
  snapshot?: ClaudeSessionSnapshot | null;
  cwd?: string;
  onRestartWith: (overrides: RestartOverrides) => void;
}> = ({ provider, sessionId, snapshot, cwd, onRestartWith }) => {
  // The Claude transport rides on the session snapshot; 'stream' (headless
  // stream-json, no PTY) swaps in transport-aware caps — see providerCaps.ts.
  const transport = snapshot?.transport;
  const caps = capsFor(provider, transport);
  const settings = snapshot?.settings;
  const stats = deriveSessionStats(snapshot);

  const [menu, setMenu] = useState<MenuState | null>(null);
  const [models, setModels] = useState<ModelOption[] | null>(null);
  /** Model id we optimistically sent `/model` for; cleared when telemetry
   *  confirms (model label changes) or after a timeout. */
  const [switching, setSwitching] = useState<string | null>(null);
  const switchTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const modelAtSwitchRef = useRef<string | undefined>(undefined);

  // Clear the "switching…" state once the reported model actually changes
  // (statusLine catches up), so the pill returns to showing truth.
  useEffect(() => {
    if (switching && stats.model !== modelAtSwitchRef.current) {
      setSwitching(null);
      if (switchTimerRef.current) clearTimeout(switchTimerRef.current);
    }
  }, [stats.model, switching]);
  useEffect(
    () => () => {
      if (switchTimerRef.current) clearTimeout(switchTimerRef.current);
    },
    [],
  );

  const loadModels = useCallback(async () => {
    try {
      if (caps.modelSource === 'claude') {
        const res = await window.electronAPI.claudeListModels();
        // Date-stamped variants of one model shorten to the same label — keep
        // the first so the menu never shows two identical rows.
        const seen: ModelOption[] = [];
        for (const id of res.seen ?? []) {
          if (res.aliases.some((a) => a.value === id)) continue;
          const label = shortModelLabel(id) || id;
          if (seen.some((s) => s.label === label)) continue;
          seen.push({ id, label, context: id.includes('[1m]') ? '1M' : '200K', seen: true });
        }
        setModels([
          ...res.aliases.map((a) => ({ id: a.value, label: a.label, context: a.context })),
          ...seen,
        ]);
      } else {
        const res = await window.electronAPI.providerListModels(
          provider as 'codex' | 'opencode' | 'pi',
          cwd,
        );
        setModels(res.map((m) => ({ id: m.id, label: m.label || m.id })));
      }
    } catch {
      setModels([]);
    }
  }, [caps.modelSource, provider, cwd]);

  const openMenu = (kind: MenuKind) => (e: React.MouseEvent<HTMLButtonElement>) => {
    const rect = e.currentTarget.getBoundingClientRect();
    // Anchor at the pill's top edge; the menu flips above it (viewport clamp)
    // since the bar sits at the bottom of the pane.
    setMenu({ kind, x: rect.left, y: rect.top - 4 });
    if (kind === 'model' && models === null) void loadModels();
  };

  // Live model switch. Claude: the `/model` slash command through the normal
  // message path. Managed (codex): claudemon's `/sessions/:id/model`, which
  // applies `thread/settings/update` to the running thread. Either way the
  // pill shows "switching…" until telemetry reports the new model. When the
  // daemon says it can't be done live (rollout fallback, opencode/pi), reopen
  // the menu as the restart confirm with its reason — same flow as the
  // permission pill.
  const liveModelSwitch = useCallback(
    (id: string, label: string, at: { x: number; y: number }) => {
      if (!sessionId) return;
      modelAtSwitchRef.current = stats.model;
      setSwitching(id);
      if (switchTimerRef.current) clearTimeout(switchTimerRef.current);
      switchTimerRef.current = setTimeout(() => setSwitching(null), 15_000);
      // PTY Claude only: `/model` is a TUI slash command, typed through the
      // normal message path. A stream-transport (headless) Claude session has
      // no TUI to interpret it — the text would land as a literal prompt — so
      // it takes the structural endpoint below like the managed providers.
      if (caps.modelSource === 'claude' && transport !== 'stream') {
        window.electronAPI.claudeMessage(sessionId, `/model ${id}`).catch((err) => {
          console.warn('[ComposerControls] live model switch failed:', err);
          setSwitching(null);
        });
        return;
      }
      window.electronAPI
        .claudeSetModel(sessionId, id)
        .then((res) => {
          if (!res.ok) {
            setSwitching(null);
            setMenu({
              kind: 'model',
              x: at.x,
              y: at.y,
              confirm: { overrides: { model: id }, label, reason: res.error },
            });
          }
        })
        .catch((err) => {
          console.warn('[ComposerControls] live model switch failed:', err);
          setSwitching(null);
        });
    },
    [sessionId, stats.model, caps.modelSource, transport],
  );

  const pickRestart = (overrides: RestartOverrides, label: string) => {
    setMenu((m) => (m ? { ...m, confirm: { overrides, label } } : m));
  };

  /** Target mode id of an in-flight live permission switch. Cleared when the
   *  daemon answers — on success the snapshot already carries the new mode
   *  (main updates livePermissionMode before resolving), so no timer needed. */
  const [permSwitching, setPermSwitching] = useState<string | null>(null);

  // Live permission switch: claudemon drives and verifies it (claude:
  // shift+tab cycle against the screen; codex: adapter approval flag). When
  // the daemon says it can't be done live, reopen the menu as the restart
  // confirm with its reason — same outcome the pick would have had on a
  // restart-only provider, just better informed.
  const livePermissionSwitch = useCallback(
    (id: string, label: string, at: { x: number; y: number }) => {
      if (!sessionId) return;
      setPermSwitching(id);
      window.electronAPI
        .claudeSetPermissionMode(sessionId, id)
        .then((res) => {
          if (!res.ok) {
            setMenu({
              kind: 'permission',
              x: at.x,
              y: at.y,
              confirm: { overrides: { permissionMode: id }, label, reason: res.error },
            });
          }
        })
        .catch((err) => {
          console.warn('[ComposerControls] live permission switch failed:', err);
        })
        .finally(() => setPermSwitching(null));
    },
    [sessionId],
  );

  // ── Pill labels ──
  const modelLabel = switching ? `${switching}…` : (stats.model ?? settings?.model ?? 'Model');
  const effortLevel = caps.effort?.levels.find((l) => l.id === settings?.effort);
  const effortLabel = effortLevel?.label ?? settings?.effort ?? 'Effort';
  // Live mode (hook telemetry — follows shift+tab in the TUI) wins over the
  // requested-at-spawn setting, same precedence as the model pill.
  const currentPermMode = snapshot?.livePermissionMode ?? settings?.permissionMode;
  const permLabel = permSwitching
    ? `${permissionModeLabel(provider, permSwitching)}…`
    : permissionModeLabel(provider, currentPermMode);

  const disabled = !sessionId;

  return (
    <span style={{ display: 'inline-flex', alignItems: 'center', gap: 2, minWidth: 0 }}>
      <button
        className="wks-composer-ctl"
        style={{ ...pillStyle, color: switching ? colors.accent : pillStyle.color }}
        onClick={openMenu('model')}
        disabled={disabled}
        title={
          disabled
            ? 'No session yet'
            : caps.modelSwitch === 'live'
              ? 'Switch model (applies to the next turn)'
              : 'Switch model (restarts the session)'
        }
      >
        <IconModel size={13} strokeWidth={2} accent="currentColor" />
        <span style={{ overflow: 'hidden', textOverflow: 'ellipsis' }}>{modelLabel}</span>
        <span style={{ opacity: 0.7 }}>▾</span>
      </button>
      {caps.effort && (
        <>
          <Sep />
          <button
            className="wks-composer-ctl"
            style={pillStyle}
            onClick={openMenu('effort')}
            disabled={disabled}
            title={disabled ? 'No session yet' : 'Reasoning effort (restarts the session)'}
          >
            <span style={{ overflow: 'hidden', textOverflow: 'ellipsis' }}>{effortLabel}</span>
            <span style={{ opacity: 0.7 }}>▾</span>
          </button>
        </>
      )}
      <Sep />
      <button
        className="wks-composer-ctl"
        style={{ ...pillStyle, color: permSwitching ? colors.accent : pillStyle.color }}
        onClick={openMenu('permission')}
        disabled={disabled}
        title={
          disabled
            ? 'No session yet'
            : caps.permissionSwitch === 'live'
              ? 'Permission mode (applies immediately)'
              : 'Permission mode (restarts the session)'
        }
      >
        <span style={{ overflow: 'hidden', textOverflow: 'ellipsis' }}>{permLabel}</span>
        <span style={{ opacity: 0.7 }}>▾</span>
      </button>

      {menu && !menu.confirm && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          onClose={() => setMenu(null)}
          minWidth={menu.kind === 'model' ? 250 : 190}
        >
          {menu.kind === 'model' && (
            <>
              <ContextMenuLabel>
                Model{caps.modelSwitch === 'restart' ? ' · restarts session' : ''}
              </ContextMenuLabel>
              {models === null && <ContextMenuItem label="Loading…" onClick={() => {}} disabled />}
              {models !== null && models.length === 0 && (
                <ContextMenuItem label="No models found" onClick={() => {}} disabled />
              )}
              {models?.map((m, i) => {
                // Live telemetry reports concrete ids; aliases match by family
                // label (e.g. "claude-sonnet-5" ↔ Sonnet, but not Sonnet 1M
                // unless the id carries the [1m] marker).
                const cur = stats.model
                  ? m.id === stats.model ||
                    (shortModelLabel(stats.model)
                      .toLowerCase()
                      .startsWith(m.label.split(' ')[0].toLowerCase()) &&
                      stats.model.includes('[1m]') === m.id.includes('[1m]'))
                  : false;
                return (
                  <React.Fragment key={m.id}>
                    {m.seen && !models[i - 1]?.seen && (
                      <>
                        <ContextMenuSeparator />
                        <ContextMenuLabel>Recently used</ContextMenuLabel>
                      </>
                    )}
                    <ContextMenuItem
                      label={modelItemLabel(m, cur)}
                      onClick={() => {
                        if (caps.modelSwitch === 'live') {
                          const at = { x: menu.x, y: menu.y };
                          setMenu(null);
                          liveModelSwitch(m.id, m.label, at);
                        } else {
                          pickRestart({ model: m.id }, m.label);
                        }
                      }}
                    />
                  </React.Fragment>
                );
              })}
            </>
          )}
          {menu.kind === 'effort' && caps.effort && (
            <>
              <ContextMenuLabel>Reasoning effort · restarts session</ContextMenuLabel>
              {caps.effort.levels.map((l) => (
                <ContextMenuItem
                  key={l.id}
                  label={l.id === settings?.effort ? `${l.label} ✓` : l.label}
                  onClick={() => pickRestart({ effort: l.id }, `${l.label} effort`)}
                />
              ))}
            </>
          )}
          {menu.kind === 'permission' && (
            <>
              <ContextMenuLabel>
                Permissions{caps.permissionSwitch === 'restart' ? ' · restarts session' : ''}
              </ContextMenuLabel>
              {caps.permissionModes.map((m) => (
                <ContextMenuItem
                  key={m.id}
                  label={
                    m.id === (currentPermMode ?? caps.permissionModes[0]?.id)
                      ? `${m.label} ✓`
                      : m.label
                  }
                  onClick={() => {
                    if (caps.permissionSwitch === 'live') {
                      const at = { x: menu.x, y: menu.y };
                      setMenu(null);
                      livePermissionSwitch(m.id, m.label, at);
                    } else {
                      pickRestart({ permissionMode: m.id }, m.label);
                    }
                  }}
                />
              ))}
            </>
          )}
        </ContextMenu>
      )}

      {menu?.confirm && (
        <ContextMenu x={menu.x} y={menu.y} onClose={() => setMenu(null)} minWidth={230}>
          {menu.confirm.reason && <ContextMenuLabel>{menu.confirm.reason}</ContextMenuLabel>}
          <ContextMenuLabel>
            {caps.restartPreservesConversation
              ? 'Restarts and resumes this conversation'
              : 'Restarts with a fresh conversation'}
          </ContextMenuLabel>
          <ContextMenuItem
            label={`Restart with ${menu.confirm.label}`}
            onClick={() => {
              onRestartWith(menu.confirm!.overrides);
              setMenu(null);
            }}
          />
          <ContextMenuSeparator />
          <ContextMenuItem label="Cancel" onClick={() => setMenu(null)} />
        </ContextMenu>
      )}
    </span>
  );
};
