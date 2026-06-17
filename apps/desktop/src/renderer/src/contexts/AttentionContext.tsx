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
    onOpenAgent(agentId);
    setViewLevel('piloting');
    closeInbox();
  }, [onOpenAgent, setViewLevel, closeInbox]);

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

  const value: AttentionContextValue = {
    agents, activeAgentId, snapshotBySession, feed, counts, topByAgent,
    inboxOpen, openInbox, closeInbox,
    selectedSig, setSelectedSig, moveSelection, selectedItem,
    viewLevel, setViewLevel,
    approve, answer, reply, sendMessage, dismiss, snooze, openAgent, respawn, reviewFile, spawnAgent,
  };

  return <AttentionContext.Provider value={value}>{children}</AttentionContext.Provider>;
};
