import type { IntentDirectionAttempt, IntentSessionRef } from './intentWorkspace';

export type IntentReconciliationAssessment = 'observed' | 'not-observed' | 'unresolved';
/** A person's assessment never rewrites a transport receipt or observed session state. */
export interface IntentReconciliation {
  id: string;
  author: 'user';
  assessment: IntentReconciliationAssessment;
  reason: string;
  at: string;
}

export interface IntentControl {
  id: string;
  workspaceId: string;
  executionId: string;
  intentRevision: number;
  target: IntentSessionRef;
  kind: 'interrupt' | 'continue';
  author: 'user';
  text: string;
  packet: string;
  createdAt: string;
  attempts: IntentDirectionAttempt[];
  reconciliations: IntentReconciliation[];
}

export interface IntentReconciliationInput {
  reconciliationId: string;
  expectedCount: number;
  assessment: IntentReconciliationAssessment;
  reason: string;
}
export type IntentControlRequest =
  | { action: 'controls'; id: string }
  | {
      action: 'prepareControl';
      id: string;
      controlId: string;
      executionId: string;
      expectedRevision: number;
      kind: IntentControl['kind'];
      text: string;
    }
  | { action: 'sendControl'; id: string; controlId: string; attemptId: string }
  | ({ action: 'reconcileControl'; id: string; controlId: string } & IntentReconciliationInput);
export type IntentControlResponse =
  | { action: 'controls'; controls: IntentControl[] }
  | { action: 'prepareControl'; control: IntentControl; created: boolean }
  | { action: 'sendControl'; control: IntentControl; dispatched: boolean }
  | { action: 'reconcileControl'; control: IntentControl };
