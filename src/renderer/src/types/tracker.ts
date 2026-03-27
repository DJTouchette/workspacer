/** Renderer-side tracker types (mirrors main process types without Node deps) */

export interface TrackerProject {
  id: string;
  key: string;
  name: string;
  avatarUrl?: string;
  provider: string;
  accountId: string;
}

export interface TrackerIssue {
  id: string;
  key: string;
  title: string;
  description: string;
  status: string;
  statusCategory: 'todo' | 'in_progress' | 'done';
  assignee?: string;
  priority?: string;
  type: string;
  labels: string[];
  provider: string;
  accountId: string;
  projectKey: string;
  url: string;
  created: string;
  updated: string;
}

export interface TrackerAccount {
  id: string;
  provider: string;
  label: string;
  config: Record<string, string>;
  pinnedProjects: string[];
}

export interface ConfigField {
  key: string;
  label: string;
  placeholder: string;
  type: 'text' | 'url' | 'email';
  required: boolean;
}

export interface TokenField {
  label: string;
  placeholder: string;
  helpText?: string;
}

export interface ProviderInfo {
  id: string;
  name: string;
  configFields: ConfigField[];
  tokenField: TokenField;
}
