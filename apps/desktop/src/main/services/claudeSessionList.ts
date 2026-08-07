/**
 * Discover existing Claude Code sessions for a given working directory.
 * Reads JSONL transcript files from ~/.claude/projects/<encoded-path>/
 */
import * as fs from 'fs';
import * as path from 'path';
import * as os from 'os';

import { trimSuffix } from '../lib/providerParity';

export interface ClaudeSessionSummary {
  sessionId: string;
  timestamp: string;
  /** First user message or session name (truncated) */
  summary: string;
}

/** Encode a directory path the same way the Claude CLI names its per-project
 *  transcript folder: every '/', '\\' and ':' becomes '-', with NO stripping —
 *  so an absolute unix cwd '/foo/bar' encodes to '-foo-bar' (leading dash) and
 *  'C:\\foo' to 'C--foo'. This must match claudemon's `encoded_cwd`
 *  (services/claudemon/src/session/transcript.rs); stripping the leading slash
 *  (as this did before) pointed every lookup at a non-existent folder, so the
 *  resume picker came up empty on unix/macOS. A trailing separator is dropped
 *  first since a real cwd never carries one. */
function encodeDirName(dir: string): string {
  return dir.replace(/[/\\]+$/, '').replace(/[/\\:]/g, '-');
}

/**
 * encodeDirName plus the one thing the encoding does not give you on its own:
 * the guarantee that the result is a PLAIN COMPONENT. Returns null to refuse.
 *
 * capspec.unscopedByDecision excuses `claude.sessionsForDir` from bus
 * confinement on the stated grounds that "the caller's string is never opened as
 * a path". That was false: the encoder maps '/', '\' and ':' to '-' and touches
 * nothing else, so '.' and '..' survive verbatim and become a real path
 * component — path.join(~/.claude/projects, '..') is ~/.claude, and the handler
 * then enumerated every *.jsonl one level ABOVE the transcript sandbox
 * (~/.claude/history.jsonl, the user's whole prompt history). '' is the same
 * shape one level down: it names the projects dir itself.
 *
 * No real cwd encodes to any of the three ('/' encodes to '-'), so refusing them
 * costs nothing and makes the exemption's sentence true. Mirrors
 * claudeProjectDirName in services/hub/cmd/brain/discovery.go; the pairs are
 * pinned by the `projectDirNames` block of
 * contracts/path-containment-cases.json.
 */
export function claudeProjectDirName(cwd: string): string | null {
  const name = encodeDirName(cwd);
  if (name === '' || name === '.' || name === '..') return null;
  return name;
}

export function listClaudeSessionsForDir(cwd: string): ClaudeSessionSummary[] {
  const claudeDir = path.join(os.homedir(), '.claude', 'projects');
  const encoded = claudeProjectDirName(cwd);
  if (encoded === null) return [];
  const projectDir = path.join(claudeDir, encoded);

  if (!fs.existsSync(projectDir)) return [];

  const files = fs.readdirSync(projectDir).filter((f) => f.endsWith('.jsonl'));
  const sessions: ClaudeSessionSummary[] = [];

  for (const file of files) {
    // TrimSuffix, not replace(): replace removes the FIRST occurrence anywhere,
    // so 'a.jsonl.b.jsonl' became 'a.b.jsonl' here and 'a.jsonl.b' in the Go twin
    // (discovery.go strings.TrimSuffix) — two different resume ids for one
    // transcript. Worse, '.jsonlagent-x.jsonl' became 'agent-x.jsonl', which then
    // matched the subagent filter below and dropped a row the brain listed.
    const sessionId = trimSuffix(file, '.jsonl');
    // Skip subagent sessions
    if (sessionId.startsWith('agent-')) continue;

    const filePath = path.join(projectDir, file);
    try {
      const stat = fs.statSync(filePath);
      // Read first ~8KB to extract metadata without loading the whole file
      const fd = fs.openSync(filePath, 'r');
      const buf = Buffer.alloc(8192);
      const bytesRead = fs.readSync(fd, buf, 0, 8192, 0);
      fs.closeSync(fd);

      const chunk = buf.toString('utf-8', 0, bytesRead);
      const lines = chunk.split('\n').filter((l) => l.trim());

      let timestamp = stat.mtime.toISOString();
      let summary = '';

      for (const line of lines) {
        try {
          const entry = JSON.parse(line);
          if (!timestamp && entry.timestamp) {
            timestamp = entry.timestamp;
          }
          // Look for a session name (set via --name flag)
          if (entry.type === 'summary' && entry.summary) {
            summary = entry.summary.slice(0, 100);
            break;
          }
          // Look for first user message
          if (!summary && entry.type === 'user' && entry.message) {
            const msg = entry.message;
            const content =
              typeof msg.content === 'string'
                ? msg.content
                : Array.isArray(msg.content)
                  ? msg.content
                      .filter((b: any) => b.type === 'text')
                      .map((b: any) => b.text)
                      .join('\n')
                  : '';
            if (content) {
              summary = content.slice(0, 100).replace(/\n/g, ' ');
            }
          }
        } catch {
          // Skip unparseable lines
        }
      }

      if (!summary) summary = sessionId;

      sessions.push({ sessionId, timestamp, summary });
    } catch {
      // Skip unreadable files
    }
  }

  // Sort by timestamp descending (most recent first)
  sessions.sort((a, b) => new Date(b.timestamp).getTime() - new Date(a.timestamp).getTime());

  // Return top 20
  return sessions.slice(0, 20);
}
