export type PaneType = 'terminal' | 'browser' | 'notes' | 'agent';

export interface PaneConfig {
  id: string;
  type: PaneType;
  title: string;
  width: number;
  widthOverride?: number;
  shell?: string;
}
