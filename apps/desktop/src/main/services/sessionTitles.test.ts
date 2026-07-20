/**
 * Title extraction from provider transcripts. Fixture lines mirror the real
 * on-disk shapes: claude `ai-title` lines + user turns (string and block
 * content, system-reminder wrappers, tool results), codex rollout event_msg
 * user_message frames.
 */
import { describe, it, expect, beforeEach, afterAll } from 'vitest';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { extractTitle, claudeProjectDir, resetSessionTitleCaches } from './sessionTitles';

const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-titles-'));
afterAll(() => fs.rmSync(dir, { recursive: true, force: true }));
beforeEach(() => resetSessionTitleCaches());

let n = 0;
function file(lines: unknown[]): string {
  const p = path.join(dir, `t${n++}.jsonl`);
  fs.writeFileSync(p, lines.map((l) => JSON.stringify(l)).join('\n') + '\n');
  return p;
}

const user = (content: unknown, extra: Record<string, unknown> = {}) => ({
  type: 'user',
  message: { role: 'user', content },
  ...extra,
});

describe('extractTitle — claude', () => {
  it('prefers the ai-title line over the first user message', async () => {
    const p = file([
      { type: 'ai-title', aiTitle: 'screenshot-parity-gui-composer', sessionId: 'x' },
      user('please fix the thing'),
    ]);
    expect(await extractTitle(p, 'claude')).toBe('screenshot-parity-gui-composer');
  });

  it('uses the last ai-title when the conversation was re-titled', async () => {
    const p = file([
      { type: 'ai-title', aiTitle: 'first pass' },
      user('hello'),
      { type: 'ai-title', aiTitle: 'second, better title' },
    ]);
    expect(await extractTitle(p, 'claude')).toBe('second, better title');
  });

  it('falls back to the first genuine user message, first line only', async () => {
    const p = file([
      user('<system-reminder>ambient stuff</system-reminder>'),
      user('Caveat: the messages below were generated…'),
      user([{ type: 'tool_result', tool_use_id: 't1', content: 'ok' }]),
      user('Fix the sidebar cards\nSecond line detail'),
      user('later message'),
    ]);
    expect(await extractTitle(p, 'claude')).toBe('Fix the sidebar cards');
  });

  it('reads block-array user content and skips meta lines', async () => {
    const p = file([
      user('skip me', { isMeta: true }),
      user([{ type: 'text', text: '## Refactor the store' }]),
    ]);
    expect(await extractTitle(p, 'claude')).toBe('Refactor the store');
  });

  it('returns undefined for an empty/foreign file or a missing one', async () => {
    expect(await extractTitle(file([{ type: 'other' }]), 'claude')).toBeUndefined();
    expect(await extractTitle(path.join(dir, 'nope.jsonl'), 'claude')).toBeUndefined();
  });

  it('caps very long messages', async () => {
    const p = file([user('x'.repeat(400))]);
    const title = await extractTitle(p, 'claude');
    expect(title!.length).toBeLessThanOrEqual(121); // 120 + ellipsis
    expect(title!.endsWith('…')).toBe(true);
  });
});

describe('extractTitle — codex', () => {
  it('uses the first user_message event', async () => {
    const p = file([
      { timestamp: 't', type: 'session_meta', payload: { session_id: 'x', cwd: '/w' } },
      { type: 'event_msg', payload: { type: 'task_started', turn_id: 'y' } },
      { type: 'event_msg', payload: { type: 'user_message', message: 'Check the release flow' } },
      { type: 'event_msg', payload: { type: 'user_message', message: 'second' } },
    ]);
    expect(await extractTitle(p, 'codex')).toBe('Check the release flow');
  });

  it('skips synthetic angle-bracket payloads', async () => {
    const p = file([
      { type: 'event_msg', payload: { type: 'user_message', message: '<environment_context>…' } },
      { type: 'event_msg', payload: { type: 'user_message', message: 'real ask' } },
    ]);
    expect(await extractTitle(p, 'codex')).toBe('real ask');
  });
});

describe('claudeProjectDir', () => {
  it('munges slashes and dots to dashes like Claude Code does', () => {
    expect(claudeProjectDir('/home/u/Work/worky/workspacer')).toBe('-home-u-Work-worky-workspacer');
    expect(claudeProjectDir('/home/u/.paperclip/x')).toBe('-home-u--paperclip-x');
  });
});
