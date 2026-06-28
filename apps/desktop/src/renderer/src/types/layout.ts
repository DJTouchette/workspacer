/** Mirrors src/main/services/layoutService.ts. */

export interface LayoutPane {
  type: string;
  title: string;
  url?: string;
  shell?: string;
  cwd?: string;
  /** Plugin panes: the contributing plugin's id, so a restored pane can re-mint
   *  its agent-cwd-scoped token. */
  pluginId?: string;
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
