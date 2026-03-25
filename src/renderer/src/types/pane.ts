export type PaneType = 'terminal' | 'browser' | 'notes' | 'agent' | 'settings';

export interface PaneConfig {
  id: string;
  type: PaneType;
  title: string;
  width: number;
  widthOverride?: number;
  shell?: string;
  cwd?: string;
  url?: string;
  appMode?: boolean;
}
