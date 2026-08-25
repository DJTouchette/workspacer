import React, { useCallback, useEffect, useRef, useState } from 'react';
import { Cloud, CloudOff, Loader2, Moon, PowerOff, type LucideIcon } from 'lucide-react';
import { ContextMenu, ContextMenuItem, ContextMenuNote, ContextMenuSeparator } from './ContextMenu';
import { Surface } from './Surface';
import {
  NODE_PRESENTATION,
  SLEEP_NOTE,
  WAKE_COST_NOTE,
  applyNodeStateChange,
  describeSleepError,
  describeWakeError,
  isNodeRegistryAbsent,
  nodeCrashNotice,
  nodeDetailLine,
  nodeToneVar,
  nodeWakeFailureNotice,
  nodesWorthShowing,
  normalizeNode,
  sleepAffordance,
  wakeAffordance,
  type NodeState,
  type RemoteNodeView,
} from '../lib/remoteNodes';
import { ensureKeyframes } from './claude-shared';

/**
 * The Connect button — the one surface for the hub's remote worker nodes.
 *
 * A node is a machine that can be off ON PURPOSE. Before this, a machine that
 * was asleep and a machine that had died looked exactly the same from here:
 * nothing. This strip is what the user asked for — *"a button on the apps that
 * show when it can't find [a machine], like 'connect'"* — and everything about
 * how it behaves comes from `.workspacer/reports/2026-08-24-fly-wake-contract.md`.
 *
 * It renders NOTHING in three cases, and all three are the normal case for
 * almost every install:
 *
 *   - the hub has no node registry (`no provider for nodes.list` — a permission
 *     check is not a feature check, so this is detected from the error, never
 *     from `can()`);
 *   - the registry is empty;
 *   - every node is connected and none is carrying a crash notice. Nothing to
 *     report is rendered as nothing; a permanent "all good" strip is chrome
 *     nobody asked for.
 *
 * The five states get five presentations, and the two transitional ones —
 * `waking` and `stopping` — are deliberately the only ones that read as
 * PROGRESS (the busy token, a spinner, "Starting…" / "Shutting down…").
 * Collapsing either into `unreachable` is the single most likely reason this
 * would feel broken to use: a machine takes real seconds to boot and real
 * seconds to drain, and a spinner that looks identical to a hang is what makes
 * someone give up.
 *
 * THE SLEEP HALF, and why a connected machine now appears here at all. A wake
 * starts a meter. Until the sleep path there was no way to stop one from inside
 * the app, so an `available` node was "quietly fine" and rendering it would have
 * been chrome. It is not quietly fine — it is billing — and the off switch has
 * to live somewhere. So a connected node shows WHEN AND ONLY WHEN there is
 * something the viewer could do about it: the hub holds the credential and this
 * caller holds the authority (`nodesWorthShowing`). A view/triage phone, or a
 * node this hub merely observes, sees exactly what it saw before.
 *
 * The button is never offered where it would be refused — a view- or triage-tier
 * client sees the state and gets a disabled control WITH THE REASON BESIDE IT,
 * not a button that dies on tap. And because a wake starts a billable machine,
 * the cost is printed on the screen rather than hidden in a tooltip.
 *
 * Seeded once from `nodes.list` (and again on every bus reconnect, like every
 * other snapshot seed) and patched from `node.state_changed`. It does not poll.
 */

/** The backend seam. Both the Electron preload and `webBackend` provide these;
 *  an older/absent one simply means the feature is not there. */
interface NodesApi {
  nodesList?: () => Promise<{ nodes: unknown; canWake?: boolean } | null>;
  nodesWake?: (id: string) => Promise<{ ok?: boolean; node?: unknown; error?: string }>;
  /** Absent on an older preload — the sleep control simply is not offered,
   *  which is the same shape the whole feature uses for "not there". */
  nodesSleep?: (id: string) => Promise<{ ok?: boolean; node?: unknown; error?: string }>;
  onHubEvent?: (cb: (ev: { type: string; data?: unknown }) => void) => () => void;
  onHubStatus?: (cb: (s: { connected: boolean }) => void) => () => void;
}

const STATE_ICON: Record<NodeState, LucideIcon> = {
  available: Cloud,
  waking: Loader2,
  // The same spinner as `waking`: shutting down on purpose is work in progress,
  // and the animation is what keeps it from reading as a hang.
  stopping: Loader2,
  stopped: Moon,
  unreachable: CloudOff,
};

export const RemoteNodesBar: React.FC<{ style?: React.CSSProperties }> = ({ style }) => {
  const [nodes, setNodes] = useState<RemoteNodeView[] | null>(null);
  const [canWake, setCanWake] = useState(false);
  /** node id → the wake we fired and the hub hasn't answered for yet. */
  const [pending, setPending] = useState<Record<string, true>>({});
  /** node id → the sleep we fired and the hub hasn't answered for yet. Separate
   *  from `pending` so the two controls disable independently — a node
   *  mid-shutdown must not render as one mid-start. */
  const [sleepPending, setSleepPending] = useState<Record<string, true>>({});
  /** node id → the last wake or sleep failure, shown on the row until it
   *  changes state. */
  const [errors, setErrors] = useState<Record<string, string>>({});
  /** The one node currently asking "are you sure?" — a wake spends real money,
   *  so a tap opens this step rather than
   *  firing `nodes.wake` directly. Same idiom as the composer's restart
   *  confirm: an anchored ContextMenu, not a fourth kind of dialog. */
  const [confirm, setConfirm] = useState<{
    id: string;
    x: number;
    y: number;
    act: 'wake' | 'sleep';
  } | null>(null);
  const mounted = useRef(true);

  useEffect(() => {
    ensureKeyframes();
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const seed = useCallback(async () => {
    const api = window.electronAPI as unknown as NodesApi | undefined;
    if (!api?.nodesList) return; // older preload / no backend: feature absent
    try {
      const snap = await api.nodesList();
      if (!mounted.current) return;
      // null = the backend already recognised the feature as absent.
      if (!snap) {
        setNodes(null);
        return;
      }
      const rows = Array.isArray(snap.nodes) ? snap.nodes : [];
      setNodes(rows.map(normalizeNode).filter((n): n is RemoteNodeView => n !== null));
      setCanWake(snap.canWake === true);
    } catch (err) {
      if (!mounted.current) return;
      // THE gotcha: `can('nodes.list')` is true even on a hub with no registry
      // at all, so the error text is the only honest feature check there is.
      // Absent → render nothing, silently. Anything else is a real failure and
      // is left to the console: this strip has no standing to own a hub outage,
      // and a toast on every reconnect blip would be worse than saying nothing.
      if (!isNodeRegistryAbsent(err)) {
        console.warn('[nodes] nodes.list failed:', err instanceof Error ? err.message : err);
      }
      setNodes(null);
    }
  }, []);

  useEffect(() => {
    void seed();
    const api = window.electronAPI as unknown as NodesApi | undefined;
    // Patch from the event; never poll. The hub publishes only on a real
    // change, so every event that arrives is worth acting on.
    const offEvent = api?.onHubEvent?.((ev) => {
      if (ev.type !== 'node.state_changed') return;
      const incoming = normalizeNode((ev.data as { node?: unknown } | undefined)?.node);
      if (!incoming) return;
      setNodes((prev) => (prev === null ? [incoming] : applyNodeStateChange(prev, incoming)));
      // The hub has spoken about this node; our optimistic pending flag and any
      // stale wake error are both superseded.
      setPending((p) => {
        if (!p[incoming.id]) return p;
        const next = { ...p };
        delete next[incoming.id];
        return next;
      });
      setSleepPending((p) => {
        if (!p[incoming.id]) return p;
        const next = { ...p };
        delete next[incoming.id];
        return next;
      });
      setErrors((e) => {
        if (!e[incoming.id]) return e;
        const next = { ...e };
        delete next[incoming.id];
        return next;
      });
    });
    // Reseed on every bus (re)connect: the hub keeps no node state across a
    // restart, so a reconnect is exactly when our copy is most likely wrong.
    let first = true;
    const offStatus = api?.onHubStatus?.((s) => {
      if (first) {
        first = false;
        return;
      }
      if (s.connected) void seed();
    });
    return () => {
      offEvent?.();
      offStatus?.();
    };
  }, [seed]);

  const wake = useCallback(async (id: string) => {
    const api = window.electronAPI as unknown as NodesApi | undefined;
    if (!api?.nodesWake) return;
    setPending((p) => ({ ...p, [id]: true }));
    setErrors((e) => {
      const next = { ...e };
      delete next[id];
      return next;
    });
    try {
      const res = await api.nodesWake(id);
      if (!mounted.current) return;
      const node = normalizeNode(res?.node);
      if (node) {
        setNodes((prev) => (prev === null ? [node] : applyNodeStateChange(prev, node)));
      }
      if (res && res.ok === false) {
        setErrors((e) => ({ ...e, [id]: describeWakeError(res.error) }));
      }
    } catch (err) {
      if (!mounted.current) return;
      setErrors((e) => ({ ...e, [id]: describeWakeError(err) }));
    } finally {
      if (mounted.current) {
        setPending((p) => {
          const next = { ...p };
          delete next[id];
          return next;
        });
      }
    }
  }, []);

  /** Put a node to sleep. Only an id crosses the seam — the signal and the
   *  drain window are the hub's, and there is deliberately nothing here that
   *  could name either. */
  const sleep = useCallback(async (id: string) => {
    const api = window.electronAPI as unknown as NodesApi | undefined;
    if (!api?.nodesSleep) return;
    setSleepPending((p) => ({ ...p, [id]: true }));
    setErrors((e) => {
      const next = { ...e };
      delete next[id];
      return next;
    });
    try {
      const res = await api.nodesSleep(id);
      if (!mounted.current) return;
      const node = normalizeNode(res?.node);
      if (node) {
        setNodes((prev) => (prev === null ? [node] : applyNodeStateChange(prev, node)));
      }
      if (res && res.ok === false) {
        setErrors((e) => ({ ...e, [id]: describeSleepError(res.error) }));
      }
    } catch (err) {
      if (!mounted.current) return;
      setErrors((e) => ({ ...e, [id]: describeSleepError(err) }));
    } finally {
      if (mounted.current) {
        setSleepPending((p) => {
          const next = { ...p };
          delete next[id];
          return next;
        });
      }
    }
  }, []);

  const requestAct = useCallback((id: string, act: 'wake' | 'sleep', x: number, y: number) => {
    setConfirm({ id, x, y, act });
  }, []);
  const cancelAct = useCallback(() => setConfirm(null), []);
  const confirmAct = useCallback(
    (id: string, act: 'wake' | 'sleep') => {
      setConfirm(null);
      if (act === 'sleep') void sleep(id);
      else void wake(id);
    },
    [wake, sleep],
  );

  if (!nodes) return null;
  // A node that is quietly fine says nothing — except that a CONNECTED machine
  // this caller can switch off is not quietly fine, it is billing, and the off
  // switch has to live somewhere. One that came back from a crash is
  // `available` AND carrying the only notice of that crash, so it counts too.
  const canSleep =
    canWake && typeof (window.electronAPI as unknown as NodesApi)?.nodesSleep === 'function';
  const shown = nodesWorthShowing(nodes, canSleep);
  if (!shown.length) return null;

  return (
    <div
      style={{
        display: 'flex',
        flexDirection: 'column',
        gap: 6,
        padding: '2px 12px 6px',
        ...style,
      }}
    >
      {shown.map((node) => (
        <NodeRow
          key={node.id}
          node={node}
          canWake={canWake}
          canSleep={canSleep}
          pending={!!pending[node.id]}
          sleepPending={!!sleepPending[node.id]}
          error={errors[node.id]}
          confirming={confirm?.id === node.id ? confirm : null}
          onRequestAct={requestAct}
          onCancelAct={cancelAct}
          onConfirmAct={confirmAct}
        />
      ))}
    </div>
  );
};

const NodeRow: React.FC<{
  node: RemoteNodeView;
  canWake: boolean;
  canSleep: boolean;
  pending: boolean;
  sleepPending: boolean;
  error?: string;
  confirming: { id: string; x: number; y: number; act: 'wake' | 'sleep' } | null;
  onRequestAct: (id: string, act: 'wake' | 'sleep', x: number, y: number) => void;
  onCancelAct: () => void;
  onConfirmAct: (id: string, act: 'wake' | 'sleep') => void;
}> = ({
  node,
  canWake,
  canSleep,
  pending,
  sleepPending,
  error,
  confirming,
  onRequestAct,
  onCancelAct,
  onConfirmAct,
}) => {
  const p = NODE_PRESENTATION[node.state];
  const tone = nodeToneVar(p.tone);
  const Icon = STATE_ICON[node.state];
  const affordance = wakeAffordance(node, canWake, pending);
  const sleepable = sleepAffordance(node, canSleep, sleepPending);
  const crash = nodeCrashNotice(node);
  const failures = nodeWakeFailureNotice(node);
  const btnRef = useRef<HTMLButtonElement>(null);
  const sleepBtnRef = useRef<HTMLButtonElement>(null);

  return (
    <Surface
      elevation="raised"
      pad="sm"
      radius="md"
      tone={tone}
      data-testid={`remote-node-${node.id}`}
      data-node-state={node.state}
      style={{ display: 'flex', flexDirection: 'column', gap: 5 }}
    >
      {/* Identity row: which machine, and what it is doing. */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 6, minWidth: 0 }}>
        <span
          data-node-tone={p.tone}
          title={p.label}
          style={{
            display: 'inline-flex',
            alignItems: 'center',
            gap: 4,
            flexShrink: 0,
            color: tone,
          }}
        >
          <Icon
            size={12}
            strokeWidth={2}
            // Motion is the difference between "booting" and "hung". Only
            // `waking` animates, and it is the only state that should.
            {...(p.progress
              ? { style: { animation: 'claudeSpinner 1s linear infinite', flexShrink: 0 } }
              : { style: { flexShrink: 0 } })}
          />
        </span>
        <span
          title={node.label}
          style={{
            fontSize: '0.72rem',
            fontWeight: 500,
            color: 'var(--wks-text-primary)',
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
            minWidth: 0,
          }}
        >
          {node.label}
        </span>
        <span
          style={{
            marginLeft: 'auto',
            flexShrink: 0,
            fontFamily: 'var(--wks-font-mono)',
            fontSize: '0.6rem',
            letterSpacing: '0.04em',
            textTransform: 'uppercase',
            color: tone,
          }}
        >
          {p.label}
        </span>
      </div>

      {/* The hub's own sentence — it is written to be read by a person. */}
      <div
        style={{
          fontSize: '0.66rem',
          lineHeight: 1.5,
          color: 'var(--wks-text-secondary)',
        }}
      >
        {nodeDetailLine(node)}
      </div>

      {/* The node telling you its last run crashed. The ONLY notice anyone gets. */}
      {crash && (
        <div style={{ fontSize: '0.66rem', lineHeight: 1.5, color: 'var(--wks-warning)' }}>
          {crash}
        </div>
      )}

      {/* Failed wakes, priced honestly. Not "left billing": the hub stops a
          machine whose wake never produced a provider. */}
      {failures && (
        <div style={{ fontSize: '0.66rem', lineHeight: 1.5, color: 'var(--wks-warning)' }}>
          {failures}
        </div>
      )}

      {error && (
        <div style={{ fontSize: '0.66rem', lineHeight: 1.5, color: 'var(--wks-error)' }}>
          {error}
        </div>
      )}

      {affordance.visible && (
        <>
          <button
            ref={btnRef}
            type="button"
            disabled={!affordance.enabled}
            title={affordance.title}
            onClick={() => {
              // A disabled control never reaches here at all, so there is no
              // path from "refused" to "confirmable" — the guard that keeps
              // view/triage tiers and credential-less nodes safe stays intact.
              if (!affordance.enabled) return;
              const r = btnRef.current?.getBoundingClientRect();
              onRequestAct(node.id, 'wake', r ? r.left : 0, r ? r.bottom + 4 : 0);
            }}
            style={{
              width: '100%',
              padding: '5px 10px',
              fontFamily: 'inherit',
              fontSize: '0.72rem',
              fontWeight: 600,
              borderRadius: 'var(--wks-radius-md)',
              border: `1px solid ${
                affordance.enabled ? 'var(--wks-accent-glow)' : 'var(--wks-border-subtle)'
              }`,
              background: affordance.enabled ? 'var(--wks-accent-bg)' : 'transparent',
              color: affordance.enabled ? 'var(--wks-accent-text)' : 'var(--wks-text-disabled)',
              cursor: affordance.enabled ? 'pointer' : 'not-allowed',
              transition: 'background 0.12s, border-color 0.12s',
            }}
          >
            {affordance.label}
          </button>
          {/* The reason a disabled control is disabled must survive without a
              hover — there is no hover on a phone, and /app runs on one. */}
          {affordance.reason && (
            <div
              style={{
                fontSize: '0.6rem',
                lineHeight: 1.5,
                textAlign: 'center',
                color: 'var(--wks-text-faint)',
              }}
            >
              {affordance.reason}
            </div>
          )}
          {/* Tapping this spends money, and the meter runs until something
              switches the machine off. That belongs on the screen, not in a
              title attribute. */}
          {affordance.enabled && (
            <div
              style={{
                fontSize: '0.6rem',
                lineHeight: 1.5,
                color: 'var(--wks-text-faint)',
              }}
            >
              {WAKE_COST_NOTE}
            </div>
          )}
        </>
      )}
      {/* The off switch. Only ever rendered for a machine that is actually up —
          a connected node, or an `unreachable` one the hub says may STILL be
          running, which is the case with a meter attached and the precise
          reason this button exists. */}
      {sleepable.visible && (
        <>
          <button
            ref={sleepBtnRef}
            type="button"
            disabled={!sleepable.enabled}
            title={sleepable.title}
            data-testid={`node-sleep-${node.id}`}
            onClick={() => {
              if (!sleepable.enabled) return;
              const r = sleepBtnRef.current?.getBoundingClientRect();
              onRequestAct(node.id, 'sleep', r ? r.left : 0, r ? r.bottom + 4 : 0);
            }}
            style={{
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
              gap: 5,
              width: '100%',
              padding: '5px 10px',
              fontFamily: 'inherit',
              fontSize: '0.72rem',
              fontWeight: 600,
              borderRadius: 'var(--wks-radius-md)',
              // Deliberately NOT the accent treatment the Connect button wears.
              // This one ends work; it should not read as the inviting action
              // on the card, and it must not sit one mis-tap away from looking
              // like the same button.
              border: `1px solid ${
                sleepable.enabled ? 'var(--wks-border-strong)' : 'var(--wks-border-subtle)'
              }`,
              background: 'transparent',
              color: sleepable.enabled ? 'var(--wks-text-secondary)' : 'var(--wks-text-disabled)',
              cursor: sleepable.enabled ? 'pointer' : 'not-allowed',
              transition: 'border-color 0.12s, color 0.12s',
            }}
          >
            <PowerOff size={11} strokeWidth={2} style={{ flexShrink: 0 }} />
            {sleepable.label}
          </button>
          {sleepable.reason && (
            <div
              style={{
                fontSize: '0.6rem',
                lineHeight: 1.5,
                textAlign: 'center',
                color: 'var(--wks-text-faint)',
              }}
            >
              {sleepable.reason}
            </div>
          )}
        </>
      )}
      {/* The confirm step: names the consequence, not the action. Same idiom
          as the composer's restart confirm — an anchored ContextMenu with the
          reason as prose, one confirming item, one cancel.

          The sleep copy names the WORK rather than the saving, because the
          money is why somebody presses it and the work is what they need to be
          warned about. */}
      {confirming && (
        <ContextMenu x={confirming.x} y={confirming.y} onClose={onCancelAct} minWidth={230}>
          <ContextMenuNote>
            {confirming.act === 'sleep' ? SLEEP_NOTE : WAKE_COST_NOTE}
          </ContextMenuNote>
          <ContextMenuItem
            label={confirming.act === 'sleep' ? `Shut down ${node.label}` : `Connect ${node.label}`}
            onClick={() => onConfirmAct(node.id, confirming.act)}
          />
          <ContextMenuSeparator />
          <ContextMenuItem label="Cancel" onClick={onCancelAct} />
        </ContextMenu>
      )}
    </Surface>
  );
};

export default RemoteNodesBar;
