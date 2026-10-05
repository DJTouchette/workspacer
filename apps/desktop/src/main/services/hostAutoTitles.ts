/** Explicit bus launches have one host owner, even with several renderers watching.
 * The per-session journal claims an attempt before inference, so a host restart
 * cannot repeat a paid request whose response was lost. No transcript is stored.
 */
import fs from 'node:fs';
import path from 'node:path';
import { createHash, randomUUID } from 'node:crypto';
import { atomicWriteFileSync } from '../lib/atomicWriteFile';
import { withConfigLock } from '../lib/configLock';
import { configService, getConfigDir } from './configService';
import type { TitleRequest } from './agentTitler';
import type { AgentProvider } from './agentProviders';

export interface HostTitle {
  state: 'pending' | 'titled' | 'skipped';
  title?: string;
}
type Record = HostTitle & { generation: string; attempted?: boolean };
type Row = {
  sessionId: string;
  hub?: string;
  label?: string;
  provider?: string;
  status: string;
  ambientState: string;
  conversation: { role: string; content?: string; command?: unknown }[];
};
export class HostAutoTitles {
  private records = new Map<string, Record>();
  private running = new Set<string>();
  constructor(
    private directory: () => string,
    private enabled: () => boolean,
    private generate: (request: TitleRequest) => Promise<string | null>,
  ) {}
  private file(id: string): string {
    return path.join(this.directory(), createHash('sha256').update(id).digest('hex') + '.json');
  }
  private read(file: string): Record | undefined {
    try {
      const row = JSON.parse(fs.readFileSync(file, 'utf8')) as Record;
      return typeof row.generation === 'string' &&
        ['pending', 'titled', 'skipped'].includes(row.state)
        ? row
        : undefined;
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === 'ENOENT') return;
      throw error; // Never reset unreadable evidence and silently pay a second time.
    }
  }
  begin(id: string, requested: boolean, label?: string): void {
    // Every launch fences responses from the previous life, including a new label.
    this.records.delete(id);
    if (!requested || label?.trim()) return;
    const file = this.file(id);
    fs.mkdirSync(path.dirname(file), { recursive: true });
    withConfigLock(file, () => {
      const previous = this.read(file);
      const record: Record =
        previous?.state === 'titled'
          ? previous
          : {
              generation: randomUUID(),
              state: previous?.attempted || previous?.state === 'skipped' ? 'skipped' : 'pending',
            };
      atomicWriteFileSync(file, JSON.stringify(record), { mode: 0o600 });
      this.records.set(id, record);
    });
  }
  metadata(id: string): HostTitle | undefined {
    const row = this.records.get(id);
    return row ? { state: row.state, ...(row.title && { title: row.title }) } : undefined;
  }
  offer(row: Row, apply: (title: HostTitle) => void): void {
    const record = this.records.get(row.sessionId);
    if (
      !record ||
      record.state !== 'pending' ||
      row.hub ||
      row.label?.trim() ||
      this.running.has(record.generation) ||
      !this.enabled()
    )
      return;
    if (
      row.status !== 'ended' &&
      !['idle', 'waiting_approval', 'waiting_input'].includes(row.ambientState)
    )
      return;
    const index = row.conversation.findIndex(
      (t) => t.role === 'user' && !t.command && t.content?.trim(),
    );
    if (index < 0) return;
    const userMessage = row.conversation[index].content!.trim();
    const assistantReply = row.conversation
      .slice(index + 1)
      .find((t) => t.role === 'assistant' && t.content?.trim())
      ?.content?.trim();
    if (!assistantReply && row.status !== 'ended') return;
    const file = this.file(row.sessionId);
    const claimed = withConfigLock(file, () => {
      const disk = this.read(file);
      if (disk?.generation !== record.generation || disk.attempted) return false;
      atomicWriteFileSync(file, JSON.stringify({ ...record, attempted: true }), { mode: 0o600 });
      return true;
    });
    if (!claimed) return;
    this.running.add(record.generation);
    void this.generate({ userMessage, assistantReply, provider: row.provider as AgentProvider })
      .catch(() => null)
      .then((title) => {
        if (this.records.get(row.sessionId) !== record || row.label?.trim()) return;
        const next: Record = {
          generation: record.generation,
          attempted: true,
          state: title ? 'titled' : 'skipped',
          ...(title && { title }),
        };
        const committed = withConfigLock(file, () => {
          if (this.read(file)?.generation !== record.generation) return false;
          atomicWriteFileSync(file, JSON.stringify(next), { mode: 0o600 });
          return true;
        });
        if (committed) {
          this.records.set(row.sessionId, next);
          apply(this.metadata(row.sessionId)!);
        }
      })
      .catch((error) => console.warn('[hostAutoTitles] title result unavailable', error))
      .finally(() => this.running.delete(record.generation));
  }
}
export const hostAutoTitles = new HostAutoTitles(
  () => path.join(getConfigDir(), 'desktop-auto-titles'),
  () => configService.getConfig().agents?.autoTitle?.enabled !== false,
  async (request) => (await import('./agentTitler')).generateAgentTitle(request),
);
