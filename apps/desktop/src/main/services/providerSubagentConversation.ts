import {
  claudeSessionStore,
  type ClaudeSessionSnapshot,
  type ConversationTurn,
} from './claudeSessionStore';
import { claudemonSessionClient } from './claudemonSessionClient';
import {
  applyConversationItems,
  type ConversationItemWire,
} from './sessionStore/conversationApplier';

function codexSubagentSnapshot(sessionId: string, agentId: string): ClaudeSessionSnapshot | null {
  const snap = claudeSessionStore.getSnapshot(sessionId);
  if (!snap || snap.provider !== 'codex') return null;
  if (!snap.subagents.some((sub) => sub.id === agentId)) return null;
  return snap;
}

async function readCodexSubagentItems(
  sessionId: string,
  agentId: string,
): Promise<{ snap: ClaudeSessionSnapshot; items: ConversationItemWire[] } | null> {
  const snap = codexSubagentSnapshot(sessionId, agentId);
  if (!snap) return null;
  const res = await claudemonSessionClient.getSubagentConversation(sessionId, agentId);
  if (!res) return null;
  return { snap, items: res.items as ConversationItemWire[] };
}

function foldItemsToConversation(
  snap: ClaudeSessionSnapshot,
  items: ConversationItemWire[],
): ConversationTurn[] {
  const temp = {
    sessionId: snap.sessionId,
    cwd: snap.cwd,
    ptyId: snap.sessionId,
    transcriptPath: '',
    status: snap.status,
    conversation: [] as ConversationTurn[],
    activeToolCalls: [],
    completedToolCalls: [],
    fileChanges: [],
    subagents: [],
    workflows: [],
    ambientState: snap.ambientState,
    startedAt: snap.startedAt,
    lastActivity: snap.lastActivity,
    totalToolCalls: 0,
    peakContext: 0,
    usage: null,
    provider: 'codex',
    transport: 'stream',
    pendingApproval: null,
    pendingQuestions: null,
  };
  applyConversationItems(temp as Parameters<typeof applyConversationItems>[0], items, () => {});
  return temp.conversation;
}

function rawText(turn: ConversationTurn): string {
  const chunks: string[] = [];
  if (turn.content.trim()) chunks.push(turn.content);
  for (const tc of turn.toolCalls ?? []) {
    chunks.push(`⚙ ${tc.name}`);
    if (typeof tc.response === 'string' && tc.response.trim()) {
      chunks.push(`↳ ${tc.response.slice(0, 400)}`);
    }
  }
  return chunks.join('\n').trim();
}

export async function readProviderSubagentConversation(
  sessionId: string,
  agentId: string,
): Promise<ConversationTurn[] | null> {
  const res = await readCodexSubagentItems(sessionId, agentId);
  if (!res) return null;
  return foldItemsToConversation(res.snap, res.items);
}

export async function readProviderSubagentTranscript(
  sessionId: string,
  agentId: string,
): Promise<{ role: string; text: string }[] | null> {
  const conv = await readProviderSubagentConversation(sessionId, agentId);
  if (!conv) return null;
  return conv
    .map((turn) => ({ role: turn.role, text: rawText(turn) }))
    .filter((turn) => turn.text.length > 0);
}
