/**
 * The sentence that tells a supervisor how to spawn its transcript-digest
 * workers.
 *
 * It is prose, but it is the ONLY thing standing between a codex supervisor and
 * a fleet of Claude summarizers: `spawn_agent` with no provider spawns Claude,
 * so a note that named a model but no harness meant the digest workers ignored
 * the supervisor's own harness entirely — and `supervisor.summarizerModel`'s
 * claude-only `'sonnet'` default was then correct by accident. Two prompt
 * builders emit this (the PTY `facadeSpawnArgs` and its managed twin
 * `managedFacadeInstructions`) and they had already drifted once, which is why
 * it is one function with one test rather than two strings.
 */
import { describe, it, expect } from 'vitest';
import { summarizerSpawnNote } from './mcpConfig';

describe('summarizerSpawnNote', () => {
  it('always names the harness, so the workers follow their supervisor', () => {
    expect(summarizerSpawnNote('codex', 'gpt-5')).toContain('provider "codex"');
    expect(summarizerSpawnNote('claude', 'sonnet')).toContain('provider "claude"');
    expect(summarizerSpawnNote('opencode')).toContain('provider "opencode"');
  });

  it('names the model when there is one, and keeps the view tier', () => {
    const note = summarizerSpawnNote('codex', 'gpt-5');
    expect(note).toContain('model "gpt-5"');
    expect(note).toContain('toolScope "view"');
  });

  it('OMITS the model instead of inventing one when it resolves to nothing', () => {
    // An absent model is "the harness's own default" — the one value valid
    // everywhere, and strictly better than naming an id that CLI would refuse.
    const note = summarizerSpawnNote('codex');
    expect(note).not.toContain('model "');
    expect(note).toContain('Do not name a model');
    expect(note).toContain('codex uses its own default');
  });

  it('treats a blank or whitespace-only model as unset', () => {
    expect(summarizerSpawnNote('codex', '')).not.toContain('model "');
    expect(summarizerSpawnNote('codex', '   ')).not.toContain('model "');
  });
});
