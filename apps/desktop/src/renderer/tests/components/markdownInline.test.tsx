import { describe, it, expect } from 'vitest';
import { render } from '@testing-library/react';
import React from 'react';
import { Markdown, MAX_INLINE_LENGTH } from '../../src/components/markdown';

function renderMd(text: string) {
  return render(<Markdown text={text} />);
}

describe('inline markdown rendering', () => {
  it('renders bold, italic, code, and links standalone', () => {
    const { container } = renderMd('a **b** *c* `d` [e](https://x.test)');
    expect(container.querySelector('strong')?.textContent).toBe('b');
    expect(container.querySelector('em')?.textContent).toBe('c');
    expect(container.querySelector('code')?.textContent).toBe('d');
    expect(container.querySelector('span[title="https://x.test"]')?.textContent).toBe('e');
  });

  it('renders code nested inside bold without literal backticks', () => {
    const { container } = renderMd('run **`cargo build`** first');
    const strong = container.querySelector('strong');
    expect(strong).not.toBeNull();
    const code = strong!.querySelector('code');
    expect(code?.textContent).toBe('cargo build');
    // The whole paragraph must not leak the backticks as text.
    expect(container.textContent).not.toContain('`');
  });

  it('renders code nested inside italic and link labels', () => {
    const { container } = renderMd('see *the `foo` helper* and [`bar`](https://x.test)');
    const em = container.querySelector('em');
    expect(em?.querySelector('code')?.textContent).toBe('foo');
    const link = container.querySelector('span[title="https://x.test"]');
    expect(link?.querySelector('code')?.textContent).toBe('bar');
    expect(container.textContent).not.toContain('`');
  });

  it('renders double-backtick code spans without stray backticks', () => {
    const { container } = renderMd('use ``a `b` c`` here');
    const code = container.querySelector('code');
    expect(code?.textContent).toBe('a `b` c');
    // Only the backticks inside the code span survive — none leak around it.
    expect(container.textContent).toBe('use a `b` c here');
  });

  it('strips one padding space from a double-backtick span (CommonMark)', () => {
    const { container } = renderMd('a `` `x` `` b');
    expect(container.querySelector('code')?.textContent).toBe('`x`');
  });

  it('keeps code span content literal (no styling inside code)', () => {
    const { container } = renderMd('`**not bold**`');
    const code = container.querySelector('code');
    expect(code?.textContent).toBe('**not bold**');
    expect(code?.querySelector('strong')).toBeNull();
  });

  it('leaves an unclosed backtick alone', () => {
    const { container } = renderMd('a stray ` backtick');
    expect(container.querySelector('code')).toBeNull();
    expect(container.textContent).toBe('a stray ` backtick');
  });
});

/**
 * The text this renderer is handed is agent output or a file the agent wrote —
 * untrusted, because a prompt-injected agent emits whatever it was told to. An
 * unclosed `[` used to make the link alternative backtrack across the whole
 * remaining segment, quadratically: 100 KB of `[a](` froze the renderer, and the
 * markdown preview pane hands it up to 5 MB on one click.
 */
describe('inline markdown — untrusted input stays bounded', () => {
  it('renders 100 KB of unclosed links inside a time budget', () => {
    const text = '[a]('.repeat(25_000);
    const t0 = performance.now();
    const { container } = renderMd(text);
    const took = performance.now() - t0;
    // Nothing is lost — the segment renders, just as plain text.
    expect(container.textContent).toBe(text);
    expect(took, `render took ${Math.round(took)}ms`).toBeLessThan(1000);
  });

  it('falls back to plain text above the per-segment cap', () => {
    const long = `**bold** ${'x'.repeat(MAX_INLINE_LENGTH)}`;
    const { container } = renderMd(long);
    expect(container.querySelector('strong')).toBeNull();
    expect(container.textContent).toContain('**bold**');
  });

  it('still renders spans in a segment just under the cap', () => {
    const { container } = renderMd(`**bold** ${'x'.repeat(MAX_INLINE_LENGTH - 100)}`);
    expect(container.querySelector('strong')?.textContent).toBe('bold');
  });

  it('leaves an over-long link label as literal text', () => {
    const label = 'a'.repeat(600);
    const { container } = renderMd(`[${label}](https://x.test)`);
    expect(container.querySelector('span[title="https://x.test"]')).toBeNull();
    expect(container.textContent).toContain(label);
  });
});
