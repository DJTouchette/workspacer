import type { IntentCriterion } from './intentEvidence';

export const INTENT_ARTIFACT_LIMITS = {
  bytes: 512 * 1024,
  records: 128,
  annotations: 512,
  demonstrationSteps: 24,
  alternatives: 6,
};
export const INTENT_ARTIFACT_MIMES = [
  'image/png',
  'image/jpeg',
  'image/webp',
  'image/gif',
  'text/plain',
  'text/markdown',
  'text/html',
] as const;
export type IntentArtifactMime = (typeof INTENT_ARTIFACT_MIMES)[number];
export interface IntentArtifact {
  id: string;
  workspaceId: string;
  intentRevision: number;
  title: string;
  kind: 'image' | 'text' | 'html' | 'url';
  mimeType?: IntentArtifactMime;
  bytes: number;
  sha256: string;
  contentId?: string;
  url?: string;
  versionOf?: string;
  executionId?: string;
  criterion?: IntentCriterion;
  author: 'user';
  createdAt: string;
}
export interface IntentAnnotation {
  id: string;
  workspaceId: string;
  intentRevision: number;
  artifactId: string;
  artifactSha256: string;
  text: string;
  point?: { x: number; y: number };
  author: 'user';
  createdAt: string;
}
export interface IntentDemonstration {
  id: string;
  workspaceId: string;
  intentRevision: number;
  title: string;
  steps: { artifactId: string; sha256: string; caption: string }[];
  author: 'user';
  createdAt: string;
}
export interface IntentAlternative {
  id: string;
  title: string;
  hypothesis: string;
  artifactIds: string[];
  executionIds: string[];
}
export interface IntentAlternativeGroup {
  id: string;
  workspaceId: string;
  intentRevision: number;
  title: string;
  purpose: string;
  /** A declared exploration budget, not an automatic timer or permission to start agents. */
  budgetMinutes: number;
  alternatives: IntentAlternative[];
  author: 'user';
  createdAt: string;
}
export interface IntentAlternativeSelection {
  id: string;
  workspaceId: string;
  intentRevision: number;
  groupId: string;
  alternativeId: string;
  reason: string;
  author: 'user';
  createdAt: string;
  previousSelectionId?: string;
}
type Write = { id: string; expectedRevision: number };
export type IntentArtifactRequest =
  | { action: 'artifacts'; id: string }
  | { action: 'readArtifact'; id: string; artifactId: string }
  | (Write & {
      action: 'addArtifact';
      artifactId: string;
      title: string;
      mimeType?: IntentArtifactMime;
      dataBase64?: string;
      url?: string;
      versionOf?: string;
      executionId?: string;
      criterionId?: string;
    })
  | (Write & {
      action: 'annotateArtifact';
      annotationId: string;
      artifactId: string;
      artifactSha256: string;
      text: string;
      point?: { x: number; y: number };
    })
  | (Write & {
      action: 'createDemonstration';
      demonstrationId: string;
      title: string;
      steps: { artifactId: string; caption: string }[];
    })
  | (Write & {
      action: 'createAlternativeGroup';
      groupId: string;
      title: string;
      purpose: string;
      budgetMinutes: number;
      alternatives: IntentAlternative[];
    })
  | (Write & {
      action: 'selectAlternative';
      selectionId: string;
      groupId: string;
      alternativeId: string;
      reason: string;
      expectedSelectionId?: string;
    });
export type IntentArtifactResponse =
  | {
      action: 'artifacts';
      artifacts: IntentArtifact[];
      annotations: IntentAnnotation[];
      demonstrations: IntentDemonstration[];
      groups: IntentAlternativeGroup[];
      selections: IntentAlternativeSelection[];
    }
  | { action: 'readArtifact'; artifact: IntentArtifact; dataBase64: string }
  | { action: 'addArtifact'; artifact: IntentArtifact }
  | { action: 'annotateArtifact'; annotation: IntentAnnotation }
  | { action: 'createDemonstration'; demonstration: IntentDemonstration }
  | { action: 'createAlternativeGroup'; group: IntentAlternativeGroup }
  | { action: 'selectAlternative'; selection: IntentAlternativeSelection };
