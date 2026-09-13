export const KNOWLEDGE_ACTIONS = [
  'knowledge',
  'captureKnowledge',
  'recordFinding',
  'prepareKnowledgePromotion',
  'publishKnowledgePromotion',
  'reconcileKnowledgePromotion',
] as const;
export interface IntentKnowledgeDocument {
  path: string;
  title: string;
  sha256: string;
  bytes: number;
}
export interface IntentKnowledgeCapture extends IntentKnowledgeDocument {
  id: string;
  workspaceId: string;
  intentRevision: number;
  content: string;
  capturedAt: string;
}
export interface IntentFinding {
  id: string;
  workspaceId: string;
  intentRevision: number;
  title: string;
  observation: string;
  captureIds: string[];
  createdAt: string;
  author: 'user';
}
export interface IntentKnowledgePromotion {
  id: string;
  workspaceId: string;
  findingId: string;
  intentRevision: number;
  projectRoot: string;
  kind: 'learning' | 'context';
  path: string;
  previousSha256: string | null;
  content: string;
  createdAt: string;
  status: 'draft' | 'unknown' | 'written';
  detail: string;
}
export type IntentKnowledgeRequest =
  | { action: 'knowledge'; id: string }
  | {
      action: 'captureKnowledge';
      id: string;
      captureId: string;
      expectedRevision: number;
      path: string;
      expectedSha256: string;
    }
  | {
      action: 'recordFinding';
      id: string;
      findingId: string;
      expectedRevision: number;
      title: string;
      observation: string;
      captureIds: string[];
    }
  | {
      action: 'prepareKnowledgePromotion';
      id: string;
      proposalId: string;
      findingId: string;
      expectedRevision: number;
      kind: 'learning' | 'context';
      path?: string;
    }
  | {
      action: 'publishKnowledgePromotion' | 'reconcileKnowledgePromotion';
      id: string;
      proposalId: string;
    };
export type IntentKnowledgeResponse =
  | {
      action: 'knowledge';
      available: boolean;
      documents: IntentKnowledgeDocument[];
      captures: IntentKnowledgeCapture[];
      findings: IntentFinding[];
      proposals: IntentKnowledgePromotion[];
    }
  | { action: 'captureKnowledge'; capture: IntentKnowledgeCapture }
  | { action: 'recordFinding'; finding: IntentFinding }
  | {
      action:
        'prepareKnowledgePromotion' | 'publishKnowledgePromotion' | 'reconcileKnowledgePromotion';
      proposal: IntentKnowledgePromotion;
    };
