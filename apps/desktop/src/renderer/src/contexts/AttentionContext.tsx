import React, { createContext, useContext, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { AgentWorkspace, ViewLevel } from '../types/pane';
import type { AttentionItem, AttentionKind } from '../types/attention';
import type { ClaudeSessionSnapshot } from '../types/claudeSession';
import type { AttentionFeed } from '../hooks/useAttentionFeed';
import { resolveApproval, resolveAnswer, resolveReply } from '../lib/resolveAttention';
import { requestReviewFile } from '../lib/reviewBus';

interface AttentionContextValue {
  agents: AgentWorkspace[];
  activeAgentId: string;
  snapshotBySession: Record<string, ClaudeSessionSnapshot>;
  feed: AttentionItem[];
  counts: { total: number; needsYou: number; byKind: Record<AttentionKind, number> };
  /** agentId → that agent's most-urgent open item; shared by SideBar + FleetDeck. */
  topByAgent: Map<string, AttentionItem>;

  // Inbox drawer
  inboxOpen: boolean;
  openInbox: () => void;
  closeInbox: () => void;
  selectedSig: string | null;
  setSelectedSig: (sig: string | null) => void;
  moveSelection: (delta: number) => void;
  selectedItem: AttentionItem | null;

  // Fleet deck
  viewLevel: ViewLevel;
  setViewLevel: (v: ViewLevel) => void;

  // Actions (all by sessionId — resolve any agent without owning its pane)
  approve: (item: AttentionItem, response: 'yes' | 'no' | 'always') => void;
  answer: (item: AttentionItem, payload: { option?: number; text?: string; answers?: string[] }) => void;
  reply: (item: AttentionItem, text: string) => void;
  /** Send a free-text message straight to a session (e.g. a Fleet card's compose box). */
  sendMessage: (sessionId: string, text: string) => void;
  dismiss: (sig: string) => void;
  snooze: (sig: string, minutes: number) => void;
  /** Focus an agent's full workspace (closes the inbox, drops to piloting). */
  openAgent: (agentId: string) => void;
  /** Re-spawn a stopped agent (wires App's respawnAgent). */
  respawn: (agentId: string) => void;
  /** Reveal a changed file in the Review pane (wires reviewBus.requestReviewFile). */
  reviewFile: (cwd: string | undefined, path?: string) => void;
  /** Open the spawn-agent flow (wires App's spawn dialog). */
  spawnAgent: () => void;
}

const AttentionContext = createContext<AttentionContextValue | null>(null);

export function useAttention(): AttentionContextValue {
  const ctx = useContext(AttentionContext);
  if (!ctx) throw new Error('useAttention must be used within <AttentionProvider>');
  return ctx;
}

/** How long a snoozed card stays hidden. */
export const SNOOZE_MINUTES = 30;

interface ProviderProps {
  agents: AgentWorkspace[];
  activeAgentId: string;
  snapshotBySession: Record<string, ClaudeSessionSnapshot>;
  inboxOpen: boolean;
  openInbox: () => void;
  closeInbox: () => void;
  viewLevel: ViewLevel;
  setViewLevel: (v: ViewLevel) => void;
  onOpenAgent: (agentId: string) => void;
  /** Re-spawn a stopped agent (App's respawnAgent). */
  onRespawnAgent?: (agentId: string) => void;
  /** Open the spawn-agent flow (App's spawn dialog). */
  onSpawnAgent?: () => void;
  /** The single shared attention feed, lifted to App so the same instance
   *  drives goToNextAttention, the SideBar header, the Inbox and the Fleet. */
  attention: AttentionFeed;
  children: React.ReactNode;
}

export const AttentionProvider: React.FC<ProviderProps> = ({
  agents, activeAgentId, snapshotBySession, inboxOpen, openInbox, closeInbox,
  viewLevel, setViewLevel, onOpenAgent, onRespawnAgent, onSpawnAgent, attention, children,
}) => {
  const { items: feed, counts, topByAgent, dismiss, snooze } = attention;
  const [selectedSig, setSelectedSig] = useState<string | null>(null);
  // Remember the selected card's index so that when it resolves out from under
  // us we can advance to the NEXT item (same slot) rather than snapping to top.
  const prevIndexRef = useRef(0);

  // Latest feed, so openAgent can clear an agent's items without taking `feed`
  // as a dependency — that would rebuild the stable actions bundle on every
  // feed tick (see the actions memo note below).
  const feedRef = useRef(feed);
  feedRef.current = feed;

  // Keep a valid selection as the feed shifts. If the selected card is still
  // present, just track its index; if it resolved away, advance to the item now
  // occupying its old slot (clamped) so triage flows downward like email.
  useEffect(() => {
    if (feed.length === 0) { if (selectedSig !== null) setSelectedSig(null); prevIndexRef.current = 0; return; }
    const idx = selectedSig ? feed.findIndex((it) => it.signature === selectedSig) : -1;
    if (idx >= 0) { prevIndexRef.current = idx; return; }
    const next = Math.min(prevIndexRef.current, feed.length - 1);
    prevIndexRef.current = next;
    setSelectedSig(feed[next].signature);
  }, [feed, selectedSig]);

  const selectedItem = useMemo(
    () => feed.find((it) => it.signature === selectedSig) ?? null,
    [feed, selectedSig],
  );

  // Keep the open agent's inbox clear. openAgent clears an agent's items at the
  // moment you open it, but while you're piloting that agent it keeps working —
  // finishing, asking, hitting an approval — and each of those would re-land in
  // the inbox for the very agent on your screen. So while piloting, auto-dismiss
  // any item for the active agent as it surfaces: you're already looking at it,
  // and the live prompt still shows in its own pane. Gated on 'piloting' so the
  // Fleet/Inbox triage views still list the active agent's items normally. Items
  // for OTHER agents are untouched. `feed` already excludes dismissed items, so
  // this fires only on genuinely new items and settles immediately.
  useEffect(() => {
    if (viewLevel !== 'piloting' || !activeAgentId) return;
    for (const it of feed) {
      if (it.agentId === activeAgentId) dismiss(it.signature);
    }
  }, [feed, viewLevel, activeAgentId, dismiss]);

  const moveSelection = useCallback((delta: number) => {
    if (feed.length === 0) return;
    const idx = feed.findIndex((it) => it.signature === selectedSig);
    const base = idx < 0 ? 0 : idx;
    const next = Math.max(0, Math.min(feed.length - 1, base + delta));
    setSelectedSig(feed[next].signature);
  }, [feed, selectedSig]);

  const hasPendingQuestion = useCallback(
    (sessionId: string) => (snapshotBySession[sessionId]?.pendingQuestions?.length ?? 0) > 0,
    [snapshotBySession],
  );

  const approve = useCallback((item: AttentionItem, response: 'yes' | 'no' | 'always') => {
    resolveApproval(item.sessionId, response, hasPendingQuestion(item.sessionId));
  }, [hasPendingQuestion]);

  const answer = useCallback((item: AttentionItem, payload: { option?: number; text?: string; answers?: string[] }) => {
    resolveAnswer(item.sessionId, payload);
  }, []);

  const reply = useCallback((item: AttentionItem, text: string) => {
    if (text.trim()) resolveReply(item.sessionId, text.trim());
  }, []);

  const sendMessage = useCallback((sessionId: string, text: string) => {
    if (text.trim()) resolveReply(sessionId, text.trim());
  }, []);

  const openAgent = useCallback((agentId: string) => {
    // Opening an agent IS the triage action for it: clear that agent's inbox
    // items so they don't linger after you've gone to deal with them. The live
    // approval/question prompts still appear in the agent's own pane, and any
    // genuinely new request (different signature) will resurface here later.
    for (const it of feedRef.current) {
      if (it.agentId === agentId) dismiss(it.signature);
    }
    onOpenAgent(agentId);
    setViewLevel('piloting');
    closeInbox();
  }, [dismiss, onOpenAgent, setViewLevel, closeInbox]);

  const respawn = useCallback((agentId: string) => {
    onRespawnAgent?.(agentId);
  }, [onRespawnAgent]);

  const reviewFile = useCallback((cwd: string | undefined, path?: string) => {
    // No specific file → point the Review pane at the repo (cwd) so it loads the
    // working-tree diff; ReviewPane selects the file when a concrete path is given.
    requestReviewFile({ cwd, path: path ?? cwd ?? '' });
  }, []);

  const spawnAgent = useCallback(() => {
    onSpawnAgent?.();
  }, [onSpawnAgent]);

  // Split the context into a STABLE actions bundle and the volatile data so the
  // memo deps stay honest. Every action below is already useCallback'd, so this
  // object only changes when one of those identities does (effectively never
  // after mount) — action-only consumers don't churn on feed ticks.
  const actions = useMemo(() => ({
    openInbox, closeInbox, setSelectedSig, moveSelection, setViewLevel,
    approve, answer, reply, sendMessage, dismiss, snooze, openAgent, respawn, reviewFile, spawnAgent,
  }), [
    openInbox, closeInbox, moveSelection, setViewLevel,
    approve, answer, reply, sendMessage, dismiss, snooze, openAgent, respawn, reviewFile, spawnAgent,
  ]);

  // Public shape stays flat + backward-compatible (the attention test and every
  // consumer read these keys directly off useAttention()). Memoized so the
  // Provider value identity is stable across renders that don't change inputs.
  const value: AttentionContextValue = useMemo(() => ({
    agents, activeAgentId, snapshotBySession, feed, counts, topByAgent,
    inboxOpen, selectedSig, selectedItem, viewLevel,
    ...actions,
  }), [
    agents, activeAgentId, snapshotBySession, feed, counts, topByAgent,
    inboxOpen, selectedSig, selectedItem, viewLevel, actions,
  ]);

  return <AttentionContext.Provider value={value}>{children}</AttentionContext.Provider>;
};
