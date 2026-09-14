/** A criterion is a nonblank line in the saved intent. Identity is revision-scoped. */
export interface IntentCriterion {
  id: string;
  intentRevision: number;
  text: string;
}

export function intentCriteria(revision: number, text: string): IntentCriterion[] {
  return text
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean)
    .map((text, index) => ({
      id: `r${revision}:c${index + 1}`,
      intentRevision: revision,
      text,
    }));
}

export type IntentEvidenceAssessment = 'reported' | 'unresolved' | 'user-verified';
export interface IntentGitEvidence {
  cwd: string;
  repositoryRoot: string;
  headCommit: string;
  capturedAt: string;
  /** Capture compares tracked working files and index with this HEAD, not launch time. */
  scope: 'tracked-working-tree-against-head';
  changedFiles: string[];
  omissions: string[];
  artifactId: string;
  sha256: string;
  bytes: number;
}
export interface IntentEvidence {
  id: string;
  workspaceId: string;
  intentRevision: number;
  criterion: IntentCriterion;
  kind: 'manual' | 'git' | 'agent-report';
  author: 'user' | 'agent';
  assessment: IntentEvidenceAssessment;
  note: string;
  reference: string;
  executionId?: string;
  linkedEvidenceId?: string;
  createdAt: string;
  /** Facts captured by the host; these do not establish that a criterion passed. */
  git?: IntentGitEvidence;
}
export interface IntentReview {
  id: string;
  workspaceId: string;
  intentRevision: number;
  proposalId?: string;
  directionId?: string;
  decision: 'accept' | 'changes-requested';
  reason: string;
  evidenceIds: string[];
  author: 'user';
  createdAt: string;
}
export type IntentEvidenceRequest =
  | { action: 'evidence'; id: string }
  | { action: 'readEvidence'; id: string; evidenceId: string }
  | {
      action: 'addEvidence';
      id: string;
      expectedRevision: number;
      evidenceId: string;
      criterionId: string;
      note: string;
      reference: string;
      assessment: IntentEvidenceAssessment;
      executionId?: string;
      linkedEvidenceId?: string;
    }
  | {
      action: 'captureEvidence';
      id: string;
      expectedRevision: number;
      evidenceId: string;
      criterionId: string;
      executionId: string;
    }
  | {
      action: 'recordReview';
      id: string;
      expectedRevision: number;
      reviewId: string;
      proposalId?: string;
      decision: IntentReview['decision'];
      reason: string;
      evidenceIds: string[];
    };
export type IntentEvidenceResponse =
  | {
      action: 'evidence';
      criteria: IntentCriterion[];
      evidence: IntentEvidence[];
      reviews: IntentReview[];
    }
  | { action: 'addEvidence' | 'captureEvidence'; evidence: IntentEvidence }
  | { action: 'readEvidence'; evidence: IntentEvidence; artifact: string }
  | { action: 'recordReview'; review: IntentReview };
