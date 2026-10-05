import { afterEach, describe, expect, it, vi } from 'vitest';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
vi.mock('./configService', () => ({ configService: {}, getConfigDir: () => '' }));
import { HostAutoTitles } from './hostAutoTitles';
const dirs: string[] = [];
afterEach(() => dirs.splice(0).forEach((dir) => fs.rmSync(dir, { recursive: true, force: true })));
function rig() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-titles-'));
  dirs.push(dir);
  const generate = vi.fn().mockResolvedValue('Repair updater handoff');
  const enabled = vi.fn(() => true);
  const host = new HostAutoTitles(() => dir, enabled, generate);
  const row = {
    sessionId: 'one',
    status: 'active',
    ambientState: 'idle',
    provider: 'codex',
    label: undefined as string | undefined,
    conversation: [
      { role: 'user', content: 'repair updater' },
      { role: 'assistant', content: 'I found it' },
    ],
  };
  return { dir, host, row, generate, enabled };
}
describe('explicit host auto title ownership', () => {
  it('requires opt in, waits for a reply/boundary, and makes one exact provider request', async () => {
    const { host, row, generate } = rig();
    const apply = vi.fn();
    host.offer(row, apply);
    expect(generate).not.toHaveBeenCalled();
    host.begin(row.sessionId, true);
    host.offer({ ...row, ambientState: 'streaming' }, apply);
    host.offer({ ...row, conversation: row.conversation.slice(0, 1) }, apply);
    expect(generate).not.toHaveBeenCalled();
    host.offer(row, apply);
    host.offer(row, apply);
    await vi.waitFor(() => expect(apply).toHaveBeenCalledOnce());
    expect(generate).toHaveBeenCalledExactlyOnceWith({
      userMessage: 'repair updater',
      assistantReply: 'I found it',
      provider: 'codex',
    });
    expect(host.metadata('one')).toEqual({ state: 'titled', title: 'Repair updater handoff' });
    host.offer(row, apply);
    expect(generate).toHaveBeenCalledOnce();
  });
  it('persists completed titles and claims across crashes without storing conversations', async () => {
    const { host, row, dir, generate } = rig();
    host.begin('one', true);
    host.offer(row, vi.fn());
    await vi.waitFor(() => expect(host.metadata('one')?.state).toBe('titled'));
    const restarted = new HostAutoTitles(
      () => dir,
      () => true,
      generate,
    );
    expect(restarted.metadata('one')?.title).toBe('Repair updater handoff');
    restarted.begin('one', true);
    restarted.offer(row, vi.fn());
    expect(restarted.metadata('one')?.title).toBe('Repair updater handoff');
    expect(generate).toHaveBeenCalledOnce();
    host.begin('two', true);
    generate.mockImplementation(() => new Promise(() => {}));
    host.offer({ ...row, sessionId: 'two' }, vi.fn());
    restarted.begin('two', true);
    restarted.offer({ ...row, sessionId: 'two' }, vi.fn());
    expect(restarted.metadata('two')?.state).toBe('skipped');
    expect(generate).toHaveBeenCalledTimes(2);
    for (const file of fs.readdirSync(dir))
      expect(fs.readFileSync(path.join(dir, file), 'utf8')).not.toContain('I found it');
  });
  it('refuses corrupt evidence rather than paying again', () => {
    const { host, dir } = rig();
    host.begin('one', true);
    fs.writeFileSync(path.join(dir, fs.readdirSync(dir)[0]), '{}');
    expect(() => host.begin('one', true)).toThrow('Invalid host title journal');
  });
  it('fences late responses against another launch and manual labels', async () => {
    const { host, row, generate } = rig();
    let finish!: (v: string) => void;
    generate.mockImplementation(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
    host.begin('one', true);
    const apply = vi.fn();
    host.offer(row, apply);
    host.begin('one', true, 'Manual');
    finish('Late title');
    await new Promise((resolve) => setImmediate(resolve));
    expect(apply).not.toHaveBeenCalled();
    expect(host.metadata('one')).toBeUndefined();
    host.begin('labelled', true, 'Manual');
    host.offer({ ...row, sessionId: 'labelled' }, apply);
    expect(generate).toHaveBeenCalledOnce();
  });
  it('respects config, foreign ownership and rejected inference without retries', async () => {
    const { host, row, enabled, generate } = rig();
    host.begin('one', true);
    enabled.mockReturnValue(false);
    host.offer(row, vi.fn());
    enabled.mockReturnValue(true);
    host.offer({ ...row, hub: 'peer' }, vi.fn());
    expect(generate).not.toHaveBeenCalled();
    generate.mockRejectedValue(new Error('unavailable'));
    host.offer(row, vi.fn());
    await vi.waitFor(() => expect(host.metadata('one')?.state).toBe('skipped'));
    host.offer(row, vi.fn());
    expect(generate).toHaveBeenCalledOnce();
  });
});
