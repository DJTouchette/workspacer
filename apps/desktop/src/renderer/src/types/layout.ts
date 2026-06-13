/** Mirrors src/main/services/layoutService.ts. */

export interface LayoutPane {
  type: string;
  title: string;
  url?: string;
  shell?: string;
  cwd?: string;
}

export interface LayoutTab {
  title: string;
  panes: LayoutPane[];
}

export interface LayoutAgent {
  name: string;
  cwd: string;
  model?: string;
  tabs: LayoutTab[];
}

export interface Layout {
  id: string;
  name: string;
  createdAt: string;
  agents: LayoutAgent[];
}
