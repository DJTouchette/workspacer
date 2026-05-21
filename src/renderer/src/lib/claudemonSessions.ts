/**
 * Thin client for the session-level claudemon endpoints that the L2 detail
 * overlay calls into (transcript fetch, approve/deny). Lives separately from
 * claudemonItems.ts because these endpoints predate the v2 items model — they
 * exist in v1 and we're just bridging the inbox to them.
 */

export interface TranscriptMessage {
  role: string;
  content: string | Array<{ type: string; [k: string]: unknown }> | null;
  raw?: unknown;
}

export interface Transcript {
  path: string | null;
  messages: TranscriptMessage[];
}

export interface SessionState {
  session_id: string;
  cwd: string | null;
  mode: string;
  pending: { kind: string; tool?: string; summary?: string; raw?: unknown } | null;
  started_at: string;
  updated_at: string;
  tool_calls: number;
  last_event: string | null;
}

export type ApprovalDecision = 'yes' | 'no' | 'always';

const DEFAULT_BASE = 'http://127.0.0.1:7891';

export class ClaudemonSessionsClient {
  constructor(private readonly baseUrl: string = DEFAULT_BASE) {}

  async getSession(id: string): Promise<SessionState | null> {
    const res = await fetch(`${this.baseUrl}/sessions/${id}`);
    if (res.status === 404) return null;
    if (!res.ok) throw new Error(`get session failed: ${res.status}`);
    return (await res.json()) as SessionState;
  }

  async getTranscript(id: string): Promise<Transcript> {
    const res = await fetch(`${this.baseUrl}/sessions/${id}/transcript`);
    if (!res.ok) throw new Error(`transcript failed: ${res.status}`);
    return (await res.json()) as Transcript;
  }

  /**
   * Resolve a parked permission decision. Returns the response body or
   * throws if the daemon refuses (e.g. 409 when the session isn't in
   * approval mode — the picker has already passed).
   */
  async approve(id: string, decision: ApprovalDecision, reason?: string): Promise<void> {
    const body: Record<string, unknown> = { decision };
    if (reason) body.reason = reason;
    const res = await fetch(`${this.baseUrl}/sessions/${id}/approve`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(body),
    });
    if (!res.ok) {
      const text = await res.text();
      throw new Error(`approve failed: ${res.status} ${text}`);
    }
  }

  /**
   * Send a chat message to a session. Only succeeds when the session's
   * mode is `input` (Claude is at a chat prompt). 409 otherwise — the
   * caller should surface that error to the user.
   */
  async sendMessage(id: string, text: string): Promise<void> {
    const res = await fetch(`${this.baseUrl}/sessions/${id}/message`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ text }),
    });
    if (!res.ok) {
      const body = await res.text();
      throw new Error(`message failed: ${res.status} ${body}`);
    }
  }
}
