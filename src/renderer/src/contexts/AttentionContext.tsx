import React, { createContext, useContext, useCallback, useEffect, useMemo, useState } from 'react';
import type { AgentWorkspace, ViewLevel } from '../types/pane';
import type { AttentionItem, AttentionKind } from '../types/attention';
import type { ClaudeSessionSnapshot } from '../types/claudeSession';
import { useAttentionFeed } from '../hooks/useAttentionFeed';
import { resolveApproval, resolveAnswer, resolveReply } from '../lib/resolveAttention';

interface AttentionContextValue {
  agents: AgentWorkspace[];
  activeAgentId: string;
  snapshotBySession: Record<string, ClaudeSessionSnapshot>;
  feed: AttentionItem[];
  counts: { total: number; needsYou: number; byKind: Record<AttentionKind, number> };

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
  enabledKinds?: Partial<Record<AttentionKind, boolean>>;
  children: React.ReactNode;
}

export const AttentionProvider: React.FC<ProviderProps> = ({
  agents, activeAgentId, snapshotBySession, inboxOpen, openInbox, closeInbox,
  viewLevel, setViewLevel, onOpenAgent, enabledKinds, children,
}) => {
  const { items: feed, counts, dismiss, snooze } = useAttentionFeed(snapshotBySession, agents, { enabledKinds });
  const [selectedSig, setSelectedSig] = useState<string | null>(null);

  // Keep a valid selection as the feed shifts: default to the top card, and if
  // the selected card resolves out from under us, fall back to the new top.
  useEffect(() => {
    if (feed.length === 0) { if (selectedSig !== null) setSelectedSig(null); return; }
    if (!selectedSig || !feed.some((it) => it.signature === selectedSig)) {
      setSelectedSig(feed[0].signature);
    }
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

  const value: AttentionContextValue = {
    agents, activeAgentId, snapshotBySession, feed, counts,
    inboxOpen, openInbox, closeInbox,
    selectedSig, setSelectedSig, moveSelection, selectedItem,
    viewLevel, setViewLevel,
    approve, answer, reply, sendMessage, dismiss, snooze, openAgent,
  };

  return <AttentionContext.Provider value={value}>{children}</AttentionContext.Provider>;
};
