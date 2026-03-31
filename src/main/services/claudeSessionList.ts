/**
 * Discover existing Claude Code sessions for a given working directory.
 * Reads JSONL transcript files from ~/.claude/projects/<encoded-path>/
 */
import * as fs from 'fs';
import * as path from 'path';
import * as os from 'os';

export interface ClaudeSessionSummary {
  sessionId: string;
  timestamp: string;
  /** First user message or session name (truncated) */
  summary: string;
}

/** Encode a directory path the same way Claude CLI does: replace path separators with -- */
function encodeDirName(dir: string): string {
  // Normalize to forward slashes, strip trailing slash, then replace / with -
  return dir
    .replace(/\\/g, '/')
    .replace(/^\//, '')
    .replace(/\/$/, '')
    .replace(/\//g, '-')
    .replace(/:/g, '-');
}

export function listClaudeSessionsForDir(cwd: string): ClaudeSessionSummary[] {
  const claudeDir = path.join(os.homedir(), '.claude', 'projects');
  const encoded = encodeDirName(cwd);
  const projectDir = path.join(claudeDir, encoded);

  if (!fs.existsSync(projectDir)) return [];

  const files = fs.readdirSync(projectDir).filter(f => f.endsWith('.jsonl'));
  const sessions: ClaudeSessionSummary[] = [];

  for (const file of files) {
    const sessionId = file.replace('.jsonl', '');
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
      const lines = chunk.split('\n').filter(l => l.trim());

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
            const content = typeof msg.content === 'string'
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
