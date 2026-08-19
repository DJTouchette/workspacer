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
 *  - Effort: live for both — claude via the `/effort <level>` slash command
 *    through the message path (verified: the CLI answers "Set effort level to …
 *    (this session only)"), codex via the daemon's thread/settings/update. Only
 *    claude's is unconfirmable, so the pill shows what it asked for there while
 *    codex's own thread/settings/updated wins. The "Default" row still restarts:
 *    a live switch can only *set* a level, and un-pinning back to the harness
 *    default needs a relaunch with no flag. The level Default resolves to is
 *    named rather than left blank — codex's per-model `defaultReasoningEffort`,
 *    and for claude its settings chain read at spawn (settings.defaultEffort).
 *  - Permission mode: live where the daemon can drive it (claude via the
 *    verified shift+tab cycle, codex via the adapter's approval flag); when
 *    the daemon reports the switch can't be done live, the pick falls back to
 *    the restart confirm with the daemon's reason. opencode/pi restart.
 *    'Full access' is the one mode Claude gates at launch — reachable live only
 *    if the process carries --dangerously-skip-permissions (settings.
 *    bypassAvailable), and otherwise a restart, which the row says up front
 *    rather than after a request that cannot succeed.
 *
 * Restart selections go through a confirm step whose copy says whether the
 * conversation survives (claude resumes; codex/opencode start fresh).
 * Displayed values: live telemetry (statusLine/usage model) wins, then the
 * requested-at-spawn `snapshot.settings`, then a placeholder.
 */

import React, { useCallback, useEffect, useRef, useState } from 'react';
import { Check, ChevronDown } from 'lucide-react';
import type { ClaudeSessionSnapshot } from '../../types/claudeSession';
import type { AgentProvider } from '../../types/pane';
import {
  capsFor,
  effortLevelLabel,
  permissionModeLabel,
  type EffortLevel,
} from '../../lib/providerCaps';
import { deriveSessionStats } from '../../lib/sessionStats';
import { remoteDisabledTitle } from '../../lib/federation';
import { loadModelOptions, type ModelOption } from '../../lib/modelOptions';
import { shortModelLabel } from '../../lib/modelLabel';
import { claudeColors as colors } from '../claude-shared';
import type { ClaudeProfile } from '../../../../main/shared/ipcTypes';
import {
  ContextMenu,
  ContextMenuItem,
  ContextMenuLabel,
  ContextMenuNote,
  ContextMenuSeparator,
} from '../ContextMenu';
import { IconModel } from '../wksIcons';

export interface RestartOverrides {
  model?: string;
  effort?: string;
  permissionMode?: string;
  /** Claude only: restart under another profile (another login). The resume
   *  finds the same transcript in the accounts' shared projects/ dir, so the
   *  conversation continues under the new account. */
  profileId?: string;
  /** Sent once the respawn resolves. Set by AUTOMATIC restarts (account
   *  failover) — a resume comes back idle, and nobody is present to say
   *  "keep going". The pills never set it: the user is right there. */
  continuePrompt?: string;
}

/** Context-window chip; the 1M window gets the accent treatment. */
const CtxBadge: React.FC<{ ctx: string }> = ({ ctx }) => {
  const big = ctx === '1M';
  return (
    <span
      style={{
        fontSize: '0.6rem',
        fontWeight: 700,
        padding: '1px 5px',
        borderRadius: 'var(--wks-radius-pill)',
        letterSpacing: '0.04em',
        fontFamily: 'var(--wks-font-mono)',
        flexShrink: 0,
        color: big ? 'var(--wks-accent-text)' : 'var(--wks-text-faint)',
        border: `1px solid ${
          big
            ? 'color-mix(in srgb, var(--wks-accent) 45%, transparent)'
            : 'var(--wks-border-subtle)'
        }`,
        backgroundColor: big
          ? 'color-mix(in srgb, var(--wks-accent) 12%, transparent)'
          : 'transparent',
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
      <span
        style={{
          color: 'var(--wks-success)',
          display: 'inline-flex',
          alignItems: 'center',
          flexShrink: 0,
        }}
      >
        <Check size={11} strokeWidth={2.25} />
      </span>
    )}
  </span>
);

/** Plain menu row label with a trailing ✓ mark when it's the current value. */
const checkedLabel = (label: string, current: boolean): React.ReactNode =>
  current ? (
    <span style={{ display: 'inline-flex', alignItems: 'center', gap: 5, minWidth: 0 }}>
      {label}
      <Check size={11} strokeWidth={2.25} style={{ flexShrink: 0 }} />
    </span>
  ) : (
    label
  );

/** Permission row: `checkedLabel` plus a "restarts" hint on the one mode this
 *  session can't be switched into live, so the cost is visible before the
 *  click rather than in a confirm step that appears to come out of nowhere. */
const modeItemLabel = (label: string, current: boolean, restarts: boolean): React.ReactNode =>
  restarts ? (
    <span style={{ display: 'inline-flex', alignItems: 'center', gap: 6, minWidth: 0 }}>
      {checkedLabel(label, current)}
      <span
        style={{
          fontSize: '0.6rem',
          fontWeight: 700,
          letterSpacing: '0.04em',
          textTransform: 'uppercase',
          fontFamily: 'var(--wks-font-mono)',
          color: 'var(--wks-text-faint)',
          flexShrink: 0,
        }}
      >
        restarts
      </span>
    </span>
  ) : (
    checkedLabel(label, current)
  );

type MenuKind = 'model' | 'effort' | 'permission' | 'profile';

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
  fontSize: '0.7rem',
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
  /** The profile (login) this session currently runs under; undefined = the
   *  default profile. Shown/switchable only when `canSwitchProfile`. */
  profileId?: string;
  /** Local Claude sessions only — a profile is a CLAUDE_CONFIG_DIR, which
   *  remote spawns scrub and other providers don't have. */
  canSwitchProfile?: boolean;
  onRestartWith: (overrides: RestartOverrides) => void;
}> = ({ provider, sessionId, snapshot, cwd, profileId, canSwitchProfile, onRestartWith }) => {
  // The Claude transport rides on the session snapshot; 'stream' (headless
  // stream-json, no PTY) swaps in transport-aware caps — see providerCaps.ts.
  const transport = snapshot?.transport;
  const caps = capsFor(provider, transport);
  const settings = snapshot?.settings;
  const stats = deriveSessionStats(snapshot);

  const [menu, setMenu] = useState<MenuState | null>(null);
  const [models, setModels] = useState<ModelOption[] | null>(null);
  // Claude account profiles, for the profile pill (fetched once when the pill
  // is live — the list is small and profile edits are rare).
  const [profiles, setProfiles] = useState<ClaudeProfile[] | null>(null);
  useEffect(() => {
    if (!canSwitchProfile) return;
    window.electronAPI
      .claudeProfilesList?.()
      .then((p) => setProfiles((p as ClaudeProfile[]) ?? []))
      .catch(() => setProfiles([]));
  }, [canSwitchProfile]);
  const currentProfileId = profileId ?? profiles?.find((p) => p.isDefault)?.id;
  const profileLabel =
    profiles?.find((p) => p.id === currentProfileId)?.name ?? (profileId ? 'Profile' : 'Default');
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
    setModels(await loadModelOptions(provider, caps.modelSource, cwd));
  }, [caps.modelSource, provider, cwd]);

  const openMenu = (kind: MenuKind) => (e: React.MouseEvent<HTMLButtonElement>) => {
    const rect = e.currentTarget.getBoundingClientRect();
    // Anchor at the pill's top edge; the menu flips above it (viewport clamp)
    // since the bar sits at the bottom of the pane.
    setMenu({ kind, x: rect.left, y: rect.top - 4 });
    if (
      models === null &&
      (kind === 'model' || (kind === 'effort' && (provider ?? 'claude') === 'codex'))
    )
      void loadModels();
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
      // A successful live switch must also land on the agent RECORD:
      // agent.model feeds later restarts and saved layouts (App listens).
      const recordSwitch = () =>
        window.dispatchEvent(
          new CustomEvent('agent:model-switched', { detail: { sessionId, model: id } }),
        );
      if (caps.modelSource === 'claude' && transport !== 'stream') {
        window.electronAPI
          .claudeMessage(sessionId, `/model ${id}`)
          .then((res) => {
            // claudeMessage resolves {ok:false, mode} on a 409 (session ended /
            // not taking input). Ignoring it left the pill spinning for the
            // full 15s and silently reverting — fall back to the restart
            // confirm like the structural path below (same as liveEffort.ts).
            if (!res?.ok) {
              setSwitching(null);
              setMenu({
                kind: 'model',
                x: at.x,
                y: at.y,
                confirm: {
                  overrides: { model: id },
                  label,
                  reason: res?.mode
                    ? `this session can't take input right now (${res.mode})`
                    : undefined,
                },
              });
              return;
            }
            recordSwitch();
          })
          .catch((err) => {
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
            return;
          }
          recordSwitch();
        })
        .catch((err) => {
          console.warn('[ComposerControls] live model switch failed:', err);
          setSwitching(null);
        });
    },
    [sessionId, stats.model, caps.modelSource, transport],
  );

  const pickRestart = (overrides: RestartOverrides, label: string, reason?: string) => {
    setMenu((m) => (m ? { ...m, confirm: { overrides, label, reason } } : m));
  };

  /**
   * Whether a permission mode can only be reached by restarting, even on a
   * provider whose switch is otherwise 'live'.
   *
   * Claude gates `bypassPermissions` on the launch flag, both transports: the
   * stream control protocol answers *"Cannot set permission mode to
   * bypassPermissions because the session was not launched with
   * --dangerously-skip-permissions"* (verified on the wire), and the PTY TUI
   * leaves the mode out of its shift+tab cycle for the same reason — where the
   * daemon discovers it only by cycling the session through every *other* mode
   * first. Neither is worth asking for when we already know the answer, so a
   * session we launched without the flag goes straight to the restart confirm.
   *
   * `bypassAvailable` undefined means we didn't record this row's launch (a
   * pre-existing or restored session): try live and let the daemon answer.
   */
  const needsRestartForMode = (id: string): boolean =>
    id === 'bypassPermissions' &&
    (provider ?? 'claude') === 'claude' &&
    settings?.bypassAvailable === false;

  const BYPASS_RESTART_REASON =
    'Full access can only be granted at launch — this session was started with approvals on.';

  /**
   * Carry the session's current permission mode and effort through a restart
   * that isn't about them. The spawn resolver reads an absent mode as 'default'
   * and an absent effort as "no override", so a model restart would otherwise
   * drop the session out of Full access — or out of a level it was live-switched
   * to — with nothing in the UI saying it had.
   *
   * Only values the provider can be *launched* with are carried: live telemetry
   * reports permission ids outside the restart vocabulary ('auto', 'dontAsk'),
   * and `/effort` accepts levels the `--effort` flag rejects ('ultracode',
   * 'auto'). Either would reach the CLI as invalid argv.
   */
  const withCurrentSettings = (o: RestartOverrides): RestartOverrides => {
    const next = { ...o };
    if (
      next.permissionMode === undefined &&
      currentPermMode &&
      caps.permissionModes.some((m) => m.id === currentPermMode)
    ) {
      next.permissionMode = currentPermMode;
    }
    if (
      next.effort === undefined &&
      currentEffort &&
      (caps.effort?.levels ?? []).some((l) => l.id === currentEffort)
    ) {
      next.effort = currentEffort;
    }
    return next;
  };

  /** Target mode id of an in-flight live permission switch. Cleared when the
   *  daemon answers — on success the snapshot already carries the new mode
   *  (main updates livePermissionMode before resolving), so no timer needed. */
  const [permSwitching, setPermSwitching] = useState<string | null>(null);
  /** Target level of an in-flight live effort switch, same contract. */
  const [effortSwitching, setEffortSwitching] = useState<string | null>(null);

  // Live effort switch. Claude submits `/effort <level>` through the message
  // path (a real command — the CLI answers "Set effort level to … (this session
  // only)"); codex goes structural via the daemon. A refusal — a busy claude, a
  // codex rollout fallback — reopens the menu as the restart confirm with the
  // reason, the same degradation as the model and permission pills.
  const liveEffortSwitch = useCallback(
    (id: string, label: string, at: { x: number; y: number }) => {
      if (!sessionId) return;
      setEffortSwitching(id);
      window.electronAPI
        .claudeSetEffort(sessionId, id)
        .then((res) => {
          if (!res.ok) {
            setMenu({
              kind: 'effort',
              x: at.x,
              y: at.y,
              confirm: { overrides: { effort: id }, label: `${label} effort`, reason: res.error },
            });
          }
        })
        .catch((err) => {
          console.warn('[ComposerControls] live effort switch failed:', err);
        })
        .finally(() => setEffortSwitching(null));
    },
    [sessionId],
  );

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
  // An omitted/empty effort means "use this harness's configured default".
  // Make that a visible selected state instead of leaving the pill as the bare
  // placeholder "Effort" with no checked menu row.
  // Precedence, strongest first: the provider's own confirmation (codex's
  // thread/settings/updated, which also catches a change made in its TUI), then
  // the level a live switch asked for (all claude has), then the spawn request.
  const currentEffort =
    snapshot?.statusLine?.effort?.trim() ||
    snapshot?.liveEffort?.trim() ||
    settings?.effort?.trim() ||
    undefined;
  const reportedModel = stats.model ?? settings?.model;
  const currentModel = models?.find((model) =>
    reportedModel ? model.id === reportedModel : model.default,
  );
  const effortLevels: EffortLevel[] =
    (provider ?? 'claude') === 'codex' && currentModel?.effortLevels?.length
      ? currentModel.effortLevels.map((id) => ({ id, label: effortLevelLabel(id) }))
      : (caps.effort?.levels ?? []);
  const effortLevel = effortLevels.find((l) => l.id === currentEffort);
  // What "Default" actually resolves to, so the pill can name a level instead of
  // the word. Two different sources because the two harnesses expose it
  // differently: Claude's comes from its settings chain, read at spawn
  // (claudeEffortDefault.ts — the CLI reports the effective effort in no
  // telemetry channel at all); Codex reports it per model on the live catalog
  // row, so it's only known once that list has loaded.
  const resolvedDefaultEffort =
    (provider ?? 'claude') === 'codex' ? currentModel?.defaultEffort : settings?.defaultEffort;
  const defaultEffortLabel = resolvedDefaultEffort
    ? effortLevelLabel(resolvedDefaultEffort)
    : undefined;
  const effortLabel = effortSwitching
    ? `${effortLevelLabel(effortSwitching)}…`
    : (effortLevel?.label ?? currentEffort ?? defaultEffortLabel ?? 'Default');
  // Live mode (hook telemetry — follows shift+tab in the TUI) wins over the
  // requested-at-spawn setting, same precedence as the model pill.
  const currentPermMode = snapshot?.livePermissionMode ?? settings?.permissionMode;
  const permLabel = permSwitching
    ? `${permissionModeLabel(provider, permSwitching)}…`
    : permissionModeLabel(provider, currentPermMode);

  // Federation: model/effort/permission switches act on the LOCAL daemon (IPC
  // or a local respawn), which can't reach a session living on a peer hub —
  // so the pills disable for remote agents. Message/interrupt/approvals are
  // unaffected: those routes go over the bus and live elsewhere.
  const remoteHub = snapshot?.hub;
  const disabled = !sessionId || !!remoteHub;
  const disabledTitle = remoteHub ? remoteDisabledTitle(remoteHub) : 'No session yet';

  return (
    <span style={{ display: 'inline-flex', alignItems: 'center', gap: 2, minWidth: 0 }}>
      <button
        className="wks-composer-ctl"
        style={{ ...pillStyle, color: switching ? colors.accent : pillStyle.color }}
        onClick={openMenu('model')}
        disabled={disabled}
        title={
          disabled
            ? disabledTitle
            : caps.modelSwitch === 'live'
              ? 'Switch model (applies to the next turn)'
              : 'Switch model (restarts the session)'
        }
      >
        <IconModel size={13} strokeWidth={2} accent="currentColor" />
        <span style={{ overflow: 'hidden', textOverflow: 'ellipsis' }}>{modelLabel}</span>
        <ChevronDown size={11} strokeWidth={2.25} style={{ opacity: 0.7, flexShrink: 0 }} />
      </button>
      {caps.effort && (
        <>
          <Sep />
          <button
            className="wks-composer-ctl"
            style={{ ...pillStyle, color: effortSwitching ? colors.accent : pillStyle.color }}
            onClick={openMenu('effort')}
            disabled={disabled}
            title={
              disabled
                ? disabledTitle
                : caps.effort.switch === 'live'
                  ? 'Reasoning effort (applies to the next turn)'
                  : 'Reasoning effort (restarts the session)'
            }
          >
            <span style={{ overflow: 'hidden', textOverflow: 'ellipsis' }}>{effortLabel}</span>
            <ChevronDown size={11} strokeWidth={2.25} style={{ opacity: 0.7, flexShrink: 0 }} />
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
            ? disabledTitle
            : caps.permissionSwitch === 'live'
              ? 'Permission mode (applies immediately)'
              : 'Permission mode (restarts the session)'
        }
      >
        <span style={{ overflow: 'hidden', textOverflow: 'ellipsis' }}>{permLabel}</span>
        <ChevronDown size={11} strokeWidth={2.25} style={{ opacity: 0.7, flexShrink: 0 }} />
      </button>
      {canSwitchProfile && (
        <>
          <Sep />
          <button
            className="wks-composer-ctl"
            style={pillStyle}
            onClick={openMenu('profile')}
            disabled={disabled}
            title={
              disabled
                ? disabledTitle
                : 'Account profile (restarts the session, resuming this conversation under the other login)'
            }
          >
            <span style={{ overflow: 'hidden', textOverflow: 'ellipsis' }}>{profileLabel}</span>
            <ChevronDown size={11} strokeWidth={2.25} style={{ opacity: 0.7, flexShrink: 0 }} />
          </button>
        </>
      )}

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
                // Coerce both sides to strings before the split/includes work:
                // over the hub bus the live model can arrive non-string, and a
                // model-list row can lack a label — either would throw here and
                // blank the pane.
                const stModel = typeof stats.model === 'string' ? stats.model : '';
                const mLabel = m.label ?? '';
                const cur = stModel
                  ? m.id === stModel ||
                    (shortModelLabel(stModel)
                      .toLowerCase()
                      .startsWith(mLabel.split(' ')[0].toLowerCase()) &&
                      stModel.includes('[1m]') === m.id.includes('[1m]'))
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
              <ContextMenuLabel>
                Reasoning effort{caps.effort.switch === 'restart' ? ' · restarts session' : ''}
              </ContextMenuLabel>
              {/* Name the level Default resolves to. This row always restarts, even
                  where the concrete levels switch live: `/effort` and
                  thread/settings/update can only *set* a level, and un-pinning so
                  the session inherits from settings again needs a relaunch with
                  no flag. The chip says so. */}
              <ContextMenuItem
                label={modeItemLabel(
                  defaultEffortLabel ? `Default · ${defaultEffortLabel}` : 'Default',
                  !currentEffort,
                  caps.effort.switch === 'live',
                )}
                onClick={() =>
                  pickRestart(
                    { effort: '' },
                    'Default effort',
                    'Clearing the override needs a relaunch — a live switch can only set a level.',
                  )
                }
              />
              <ContextMenuSeparator />
              {effortLevels.map((l) => (
                <ContextMenuItem
                  key={l.id}
                  label={checkedLabel(l.label, l.id === currentEffort)}
                  onClick={() => {
                    if (caps.effort?.switch === 'live') {
                      const at = { x: menu.x, y: menu.y };
                      setMenu(null);
                      liveEffortSwitch(l.id, l.label, at);
                    } else {
                      pickRestart({ effort: l.id }, `${l.label} effort`);
                    }
                  }}
                />
              ))}
            </>
          )}
          {menu.kind === 'permission' && (
            <>
              <ContextMenuLabel>
                Permissions{caps.permissionSwitch === 'restart' ? ' · restarts session' : ''}
              </ContextMenuLabel>
              {caps.permissionModes.map((m) => {
                // Per-row, not per-provider: the menu is 'live' but this one
                // mode isn't. When the whole provider restarts, the header
                // already says so — don't repeat it on every row.
                const restarts = caps.permissionSwitch === 'live' && needsRestartForMode(m.id);
                return (
                  <ContextMenuItem
                    key={m.id}
                    label={modeItemLabel(
                      m.label,
                      m.id === (currentPermMode ?? caps.permissionModes[0]?.id),
                      restarts,
                    )}
                    onClick={() => {
                      if (restarts) {
                        pickRestart({ permissionMode: m.id }, m.label, BYPASS_RESTART_REASON);
                      } else if (caps.permissionSwitch === 'live') {
                        const at = { x: menu.x, y: menu.y };
                        setMenu(null);
                        livePermissionSwitch(m.id, m.label, at);
                      } else {
                        pickRestart({ permissionMode: m.id }, m.label);
                      }
                    }}
                  />
                );
              })}
            </>
          )}
          {menu.kind === 'profile' && (
            <>
              <ContextMenuLabel>Profile · restarts, same conversation</ContextMenuLabel>
              {profiles === null && (
                <ContextMenuItem label="Loading…" onClick={() => {}} disabled />
              )}
              {(profiles ?? []).map((p) => (
                <ContextMenuItem
                  key={p.id}
                  label={checkedLabel(
                    (p.weight ?? 0) > 0 ? `${p.name} · auto` : p.name,
                    p.id === currentProfileId,
                  )}
                  onClick={() => {
                    if (p.id === currentProfileId) {
                      setMenu(null);
                      return;
                    }
                    pickRestart({ profileId: p.id }, `the ${p.name} profile`);
                  }}
                />
              ))}
            </>
          )}
        </ContextMenu>
      )}

      {menu?.confirm && (
        <ContextMenu x={menu.x} y={menu.y} onClose={() => setMenu(null)} minWidth={230}>
          {/* A sentence, and sometimes the provider's own error text — prose,
              not a header. As a ContextMenuLabel it rendered unwrapped and
              stretched the menu to ~815px, which the viewport clamp then flipped
              left over the sidebar. */}
          {menu.confirm.reason && <ContextMenuNote>{menu.confirm.reason}</ContextMenuNote>}
          <ContextMenuLabel>
            {caps.restartPreservesConversation
              ? 'Restarts and resumes this conversation'
              : 'Restarts with a fresh conversation'}
          </ContextMenuLabel>
          <ContextMenuItem
            label={`Restart with ${menu.confirm.label}`}
            onClick={() => {
              // Every restart path funnels through here — the model/effort
              // picks, and the fallbacks from a live switch the daemon refused
              // — so this is the one place that has to keep the permission mode
              // from being reset by omission.
              onRestartWith(withCurrentSettings(menu.confirm!.overrides));
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
