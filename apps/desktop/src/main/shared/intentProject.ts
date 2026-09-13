import type { IntentWorkspace } from './intentWorkspace';

/** Intent-owned identity, independent of config.yaml's root-keyed fleet settings. */
export interface IntentRepository {
  id: string;
  projectId: string;
  root: string;
  revision: number;
  createdAt: string;
  updatedAt: string;
}
export interface IntentProject {
  id: string;
  name: string;
  revision: number;
  createdAt: string;
  updatedAt: string;
  repositories: IntentRepository[];
}
export type IntentProjectRequest =
  | { action: 'projects' }
  | { action: 'renameIntentProject'; projectId: string; expectedRevision: number; name: string }
  | { action: 'addIntentRepository'; projectId: string; expectedRevision: number; root: string }
  | {
      action: 'relocateIntentRepository';
      projectId: string;
      repositoryId: string;
      expectedRevision: number;
      root: string;
      reason: string;
    };
export type IntentProjectResponse =
  | { action: 'projects'; projects: IntentProject[] }
  | { action: 'renameIntentProject' | 'addIntentRepository'; project: IntentProject }
  | { action: 'relocateIntentRepository'; project: IntentProject; workspaces: IntentWorkspace[] };
