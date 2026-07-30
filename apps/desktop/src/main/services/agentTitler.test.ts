import { describe, it, expect } from 'vitest';
import { sanitizeTitle, buildTitlePrompt } from './agentTitler';

/**
 * The titler names an agent from raw model output, so the interesting surface
 * is everything a model does that ISN'T a title: quoting it, prefixing it,
 * apologising, or answering in a paragraph. Each of those must come back null
 * so the caller falls back to the user's own first line rather than naming an
 * agent "Sure! Here's a title for that conversation:".
 */

describe('sanitizeTitle', () => {
  it('passes a clean title through', () => {
    expect(sanitizeTitle('Fix the flaky sidebar test')).toBe('Fix the flaky sidebar test');
  });

  it('takes the first non-empty line', () => {
    expect(sanitizeTitle('\n\nAdd the working timer\n\nsome rambling after')).toBe(
      'Add the working timer',
    );
  });

  it('strips quotes, backticks, and smart quotes', () => {
    expect(sanitizeTitle('"Fix the flaky test"')).toBe('Fix the flaky test');
    expect(sanitizeTitle('`Fix the flaky test`')).toBe('Fix the flaky test');
    expect(sanitizeTitle('“Fix the flaky test”')).toBe('Fix the flaky test');
  });

  it('strips a Title: style preamble', () => {
    expect(sanitizeTitle('Title: Resize the sidebar')).toBe('Resize the sidebar');
    expect(sanitizeTitle("Here's a title: Resize the sidebar")).toBe('Resize the sidebar');
  });

  it('strips markdown bullets and headers', () => {
    expect(sanitizeTitle('- Resize the sidebar')).toBe('Resize the sidebar');
    expect(sanitizeTitle('## Resize the sidebar')).toBe('Resize the sidebar');
  });

  it('drops trailing punctuation', () => {
    expect(sanitizeTitle('Resize the sidebar.')).toBe('Resize the sidebar');
    expect(sanitizeTitle('Resize the sidebar!')).toBe('Resize the sidebar');
  });

  it('caps a long-winded title at a word boundary', () => {
    const out = sanitizeTitle(
      'Investigate and repair the intermittent failure in the sidebar resize suite',
    );
    expect(out).toBeTruthy();
    expect(out!.length).toBeLessThanOrEqual(52);
    expect(out!.split(' ').length).toBeLessThanOrEqual(7);
    // Whole words only — no mid-word truncation.
    expect(out).toBe('Investigate and repair the intermittent failure');
  });

  it('refuses a refusal', () => {
    expect(sanitizeTitle("I'm sorry, I can't help with that.")).toBeNull();
    expect(sanitizeTitle('Sorry — I need more context first')).toBeNull();
    expect(sanitizeTitle('As an AI language model, I cannot')).toBeNull();
  });

  it('refuses prose that only pretends to be a title', () => {
    const prose =
      'This conversation appears to be about a user asking the assistant to look into a ' +
      'failing test in the sidebar suite, and then discussing several possible approaches ' +
      'to fixing it before settling on one.';
    expect(sanitizeTitle(prose)).toBeNull();
  });

  it('refuses empty or whitespace-only output', () => {
    expect(sanitizeTitle('')).toBeNull();
    expect(sanitizeTitle('   \n  \n')).toBeNull();
    expect(sanitizeTitle('""')).toBeNull();
  });

  it('survives undefined-ish input', () => {
    expect(sanitizeTitle(undefined as unknown as string)).toBeNull();
  });
});

describe('buildTitlePrompt', () => {
  it('carries the user message and the reply', () => {
    const p = buildTitlePrompt('fix the flaky test', 'Looking at the suite now');
    expect(p).toContain('User: fix the flaky test');
    expect(p).toContain('Assistant: Looking at the suite now');
    expect(p).toMatch(/3 to 6 words/);
  });

  it('omits the assistant line when there is no reply', () => {
    const p = buildTitlePrompt('fix the flaky test');
    expect(p).not.toContain('Assistant:');
  });

  it('caps both sides so a pasted wall of text cannot blow up the argv', () => {
    const p = buildTitlePrompt('x'.repeat(50_000), 'y'.repeat(50_000));
    expect(p.length).toBeLessThan(2500);
  });
});
