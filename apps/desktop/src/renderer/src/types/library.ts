export type LibraryScope = 'global' | 'project' | 'claude';
/** 'command' is a Claude Code custom slash command (`.claude/commands/*.md`),
 *  surfaced read-mostly alongside skills so the composer's "/" picker can list
 *  them. 'dispatch' is a Fleet Manager dispatch template (text + optional
 *  default resultSchema, rendered host-side at spawn) — deliberately nothing
 *  else; see the main process's libraryService. */
export type LibraryKind = 'prompt' | 'skill' | 'agent' | 'mcp' | 'command' | 'dispatch';
export type LibraryAction = 'insert' | 'spawn' | 'copy';

/** Which root a claude-scoped item lives under, in the precedence order Claude
 *  Code itself resolves a name in. `plugin:<name>` items are read-only. Mirrors
 *  the main process's libraryService.ClaudeOrigin. */
export type ClaudeOrigin = 'project' | 'user' | `plugin:${string}`;

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
  /** Default structured-result contract — present only when kind === 'dispatch'. */
  resultSchema?: Record<string, unknown>;
  /** Which root a claude-scoped item came from. Absent for global/project. */
  origin?: ClaudeOrigin;
  /** False when the file belongs to something else (an installed plugin). */
  editable?: boolean;
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
  resultSchema?: Record<string, unknown>;
  /** Claude scope only — which root to write into ('project' | 'user'). */
  origin?: ClaudeOrigin;
  body: string;
  cwd?: string;
}
