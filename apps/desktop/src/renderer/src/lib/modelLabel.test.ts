import { describe, it, expect } from 'vitest';
import { shortModelLabel } from './modelLabel';

describe('shortModelLabel', () => {
  it('strips provider prefix, claude- vendor prefix, and trailing date stamp', () => {
    expect(shortModelLabel('claude-opus-4-8-20250101')).toBe('opus-4-8');
    expect(shortModelLabel('anthropic/claude-sonnet-4')).toBe('sonnet-4');
    expect(shortModelLabel('openai/gpt-5.4')).toBe('gpt-5.4');
    expect(shortModelLabel('gpt-5.4')).toBe('gpt-5.4');
  });

  it('keeps bare aliases and the [1m] marker as-is', () => {
    expect(shortModelLabel('opus')).toBe('opus');
    expect(shortModelLabel('sonnet[1m]')).toBe('sonnet[1m]');
  });

  it('returns "" for empty / missing input', () => {
    expect(shortModelLabel('')).toBe('');
    expect(shortModelLabel(undefined)).toBe('');
  });

  it('returns "" for a non-string model without throwing (hub-bus payloads are untrusted)', () => {
    // A model value deserialized from the hub bus (web/remote) is not guaranteed
    // to be a string; calling .replace on it would throw inside a render and
    // blank the agent pane. The guard must swallow these safely.
    expect(() => shortModelLabel({ toString: () => 'x' } as unknown as string)).not.toThrow();
    expect(shortModelLabel(123 as unknown as string)).toBe('');
    expect(shortModelLabel(null as unknown as string)).toBe('');
    expect(shortModelLabel({} as unknown as string)).toBe('');
  });
});
