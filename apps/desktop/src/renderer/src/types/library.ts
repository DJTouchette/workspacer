export type LibraryScope = 'global' | 'project' | 'claude';
export type LibraryKind = 'prompt' | 'skill' | 'agent' | 'mcp';
export type LibraryAction = 'insert' | 'spawn' | 'copy';

/** An MCP server definition, in Claude Code's `mcpServers` shape. Mirrors the
 *  main process's libraryService.McpServerConfig. */
export interface McpServerConfig {
  type?: 'stdio' | 'http' | 'sse';
  command?: string;
  args?: string[];
  env?: Record<string, string>;
  url?: string;
  headers?: Record<string, string>;
}

/** A reusable prompt or skill stored as a markdown file. Mirrors the main
 *  process's libraryService.LibraryItem. */
export interface LibraryItem {
  id: string;
  scope: LibraryScope;
  title: string;
  kind: LibraryKind;
  description?: string;
  tags?: string[];
  action?: LibraryAction;
  /** MCP server config — present only when kind === 'mcp'. */
  mcp?: McpServerConfig;
  body: string;
  path: string;
}

/** Payload for saving/creating an item (id derives from title if omitted). */
export interface LibrarySaveInput {
  scope: LibraryScope;
  id?: string;
  title: string;
  kind: LibraryKind;
  description?: string;
  tags?: string[];
  action?: LibraryAction;
  mcp?: McpServerConfig;
  body: string;
  cwd?: string;
}
