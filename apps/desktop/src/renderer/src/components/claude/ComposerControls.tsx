/**
 * Session control pills — "Opus 4.5 ▾ · High ▾ · Full access ▾" — rendered in
 * the pane's bottom status bar next to the context meter. What each pill can
 * do comes from lib/providerCaps.ts:
 *
 *  - Model:  claude switches live (`/model <id>` submitted through the normal
 *    message path); managed providers restart with the new model.
 *  - Effort: codex only, restart (`-c model_reasoning_effort=<level>`).
 *  - Permission mode: restart for every provider.
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
import { claudeColors as colors } from '../claude-shared';
import { ContextMenu, ContextMenuItem, ContextMenuLabel, ContextMenuSeparator } from '../ContextMenu';

export interface RestartOverrides {
  model?: string;
  effort?: string;
  permissionMode?: string;
}

interface ModelOption {
  id: string;
  label: string;
}

type MenuKind = 'model' | 'effort' | 'permission';

interface MenuState {
  kind: MenuKind;
  x: number;
  y: number;
  /** Set once the user picked a restart-requiring value — confirm view. */
  confirm?: { overrides: RestartOverrides; label: string };
}

const pillStyle: React.CSSProperties = {
  display: 'inline-flex',
  alignItems: 'center',
  gap: 4,
  padding: '1px 8px',
  borderRadius: 9,
  border: `1px solid ${colors.borderSubtle}`,
  background: 'transparent',
  color: colors.muted,
  cursor: 'pointer',
  fontSize: '0.6rem',
  fontFamily: 'inherit',
  fontWeight: 600,
  lineHeight: '16px',
  whiteSpace: 'nowrap',
  maxWidth: 160,
  overflow: 'hidden',
  textOverflow: 'ellipsis',
  flexShrink: 0,
};

export const ComposerControls: React.FC<{
  provider: AgentProvider | undefined;
  sessionId: string | null;
  snapshot?: ClaudeSessionSnapshot | null;
  cwd?: string;
  onRestartWith: (overrides: RestartOverrides) => void;
}> = ({ provider, sessionId, snapshot, cwd, onRestartWith }) => {
  const caps = capsFor(provider);
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
  useEffect(() => () => {
    if (switchTimerRef.current) clearTimeout(switchTimerRef.current);
  }, []);

  const loadModels = useCallback(async () => {
    try {
      if (caps.modelSource === 'claude') {
        const res = await window.electronAPI.claudeListModels();
        const seen = (res.seen ?? [])
          .filter((id) => !res.aliases.some((a) => a.value === id))
          .map((id) => ({ id, label: id }));
        setModels([...res.aliases.map((a) => ({ id: a.value, label: a.label })), ...seen]);
      } else {
        const res = await window.electronAPI.providerListModels(provider as 'codex' | 'opencode' | 'pi', cwd);
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

  const liveModelSwitch = useCallback((id: string) => {
    if (!sessionId) return;
    modelAtSwitchRef.current = stats.model;
    setSwitching(id);
    if (switchTimerRef.current) clearTimeout(switchTimerRef.current);
    switchTimerRef.current = setTimeout(() => setSwitching(null), 15_000);
    window.electronAPI.claudeMessage(sessionId, `/model ${id}`).catch((err) => {
      console.warn('[ComposerControls] live model switch failed:', err);
      setSwitching(null);
    });
  }, [sessionId, stats.model]);

  const pickRestart = (overrides: RestartOverrides, label: string) => {
    setMenu((m) => (m ? { ...m, confirm: { overrides, label } } : m));
  };

  // ── Pill labels ──
  const modelLabel = switching
    ? `${switching}…`
    : stats.model ?? settings?.model ?? 'Model';
  const effortLevel = caps.effort?.levels.find((l) => l.id === settings?.effort);
  const effortLabel = effortLevel?.label ?? settings?.effort ?? 'Effort';
  // Live mode (hook telemetry — follows shift+tab in the TUI) wins over the
  // requested-at-spawn setting, same precedence as the model pill.
  const currentPermMode = snapshot?.livePermissionMode ?? settings?.permissionMode;
  const permLabel = permissionModeLabel(provider, currentPermMode);

  const disabled = !sessionId;

  return (
    <span style={{ display: 'inline-flex', alignItems: 'center', gap: 5, minWidth: 0 }}>
      <button
        style={{ ...pillStyle, color: switching ? colors.accent : pillStyle.color }}
        onClick={openMenu('model')}
        disabled={disabled}
        title={disabled ? 'No session yet' : caps.modelSwitch === 'live' ? 'Switch model (applies to the next turn)' : 'Switch model (restarts the session)'}
      >
        <span style={{ overflow: 'hidden', textOverflow: 'ellipsis' }}>{modelLabel}</span>
        <span style={{ opacity: 0.7 }}>▾</span>
      </button>
      {caps.effort && (
        <button
          style={pillStyle}
          onClick={openMenu('effort')}
          disabled={disabled}
          title={disabled ? 'No session yet' : 'Reasoning effort (restarts the session)'}
        >
          <span style={{ overflow: 'hidden', textOverflow: 'ellipsis' }}>{effortLabel}</span>
          <span style={{ opacity: 0.7 }}>▾</span>
        </button>
      )}
      <button
        style={pillStyle}
        onClick={openMenu('permission')}
        disabled={disabled}
        title={disabled ? 'No session yet' : 'Permission mode (restarts the session)'}
      >
        <span style={{ overflow: 'hidden', textOverflow: 'ellipsis' }}>{permLabel}</span>
        <span style={{ opacity: 0.7 }}>▾</span>
      </button>

      {menu && !menu.confirm && (
        <ContextMenu x={menu.x} y={menu.y} onClose={() => setMenu(null)} minWidth={190}>
          {menu.kind === 'model' && (
            <>
              <ContextMenuLabel>
                Model{caps.modelSwitch === 'restart' ? ' · restarts session' : ''}
              </ContextMenuLabel>
              {models === null && <ContextMenuItem label="Loading…" onClick={() => {}} disabled />}
              {models !== null && models.length === 0 && (
                <ContextMenuItem label="No models found" onClick={() => {}} disabled />
              )}
              {models?.map((m) => (
                <ContextMenuItem
                  key={m.id}
                  label={m.label}
                  onClick={() => {
                    if (caps.modelSwitch === 'live') {
                      liveModelSwitch(m.id);
                      setMenu(null);
                    } else {
                      pickRestart({ model: m.id }, m.label);
                    }
                  }}
                />
              ))}
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
              <ContextMenuLabel>Permissions · restarts session</ContextMenuLabel>
              {caps.permissionModes.map((m) => (
                <ContextMenuItem
                  key={m.id}
                  label={m.id === (currentPermMode ?? caps.permissionModes[0]?.id) ? `${m.label} ✓` : m.label}
                  onClick={() => pickRestart({ permissionMode: m.id }, m.label)}
                />
              ))}
            </>
          )}
        </ContextMenu>
      )}

      {menu?.confirm && (
        <ContextMenu x={menu.x} y={menu.y} onClose={() => setMenu(null)} minWidth={230}>
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
