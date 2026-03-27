export type PaneType = 'terminal' | 'browser' | 'notes' | 'agent' | 'claude' | 'settings' | 'dashboard' | 'tracker' | 'devops';

export interface PaneConfig {
  id: string;
  type: PaneType;
  title: string;
  shell?: string;
  cwd?: string;
  url?: string;
  appMode?: boolean;
  hibernated?: boolean;
  profileId?: string;
}

export interface TabConfig {
  id: string;
  title: string;
  panes: PaneConfig[];
  activePaneId: string;
}
