/**
 * Issue key detection utilities for terminal link providers and Claude input.
 *
 * - Terminal: registers an xterm link provider that makes PROJ-123 clickable
 * - Claude input: detects issue keys and resolves them to context strings
 */
import type { Terminal, ILinkProvider, ILink } from '@xterm/xterm';

/** Matches issue keys like PL-43, PROJ-123, AB-1 */
export const ISSUE_KEY_REGEX = /\b[A-Z][A-Z0-9]+-\d+\b/g;

/** Extract all issue keys from a string */
export function extractIssueKeys(text: string): string[] {
  const matches = text.match(ISSUE_KEY_REGEX);
  if (!matches) return [];
  return [...new Set(matches)];
}

/** Resolve issue keys to context strings for Claude prompts */
export async function resolveIssueContext(keys: string[]): Promise<string> {
  const parts: string[] = [];
  for (const key of keys) {
    try {
      const issue = await window.electronAPI.trackerResolveIssueKey(key);
      if (issue) {
        const desc = issue.description ? `\nDescription: ${issue.description.slice(0, 500)}` : '';
        parts.push(`[${key}: ${issue.title} | Status: ${issue.status} | Type: ${issue.type}${issue.assignee ? ` | Assignee: ${issue.assignee}` : ''}${desc}]`);
      }
    } catch { /* skip unresolvable keys */ }
  }
  return parts.join('\n');
}

// ── Terminal link provider ──

export interface IssuePeekData {
  key: string;
  title: string;
  status: string;
  statusCategory: string;
  type: string;
  assignee?: string;
  description: string;
  url: string;
}

/**
 * Register an xterm link provider that detects issue keys (PROJ-123) and
 * calls onPeek when clicked. Returns a dispose function.
 */
export function registerIssueLinkProvider(
  term: Terminal,
  onPeek: (data: IssuePeekData, x: number, y: number) => void,
): { dispose: () => void } {
  const provider: ILinkProvider = {
    provideLinks(lineNumber: number, callback: (links: ILink[] | undefined) => void) {
      const line = term.buffer.active.getLine(lineNumber - 1);
      if (!line) { callback(undefined); return; }

      const text = line.translateToString();
      const links: ILink[] = [];
      let match: RegExpExecArray | null;
      const re = new RegExp(ISSUE_KEY_REGEX.source, 'g');

      while ((match = re.exec(text)) !== null) {
        const startX = match.index;
        const key = match[0];
        links.push({
          range: {
            start: { x: startX + 1, y: lineNumber },
            end: { x: startX + key.length + 1, y: lineNumber },
          },
          text: key,
          activate(_event, text) {
            // Resolve and show peek
            window.electronAPI.trackerResolveIssueKey(text).then((issue) => {
              if (issue) {
                onPeek({
                  key: issue.key,
                  title: issue.title,
                  status: issue.status,
                  statusCategory: issue.statusCategory,
                  type: issue.type,
                  assignee: issue.assignee,
                  description: issue.description?.slice(0, 300) ?? '',
                  url: issue.url,
                }, 0, 0);
              }
            }).catch(() => {});
          },
          hover(_event, text) {
            // Tooltip handled by xterm's built-in hover decoration
          },
        });
      }

      callback(links.length > 0 ? links : undefined);
    },
  };

  return term.registerLinkProvider(provider);
}
