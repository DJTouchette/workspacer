import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import React from 'react';
import { FileLink, defaultOpenTarget } from '../../src/components/claude/FileLink';
import { Markdown, MarkdownFileCwdProvider } from '../../src/components/markdown';
import { EDITOR_OPEN_FILE_EVENT } from '../../src/lib/editorBus';
import { MARKDOWN_PREVIEW_EVENT } from '../../src/lib/previewBus';

/**
 * The little icon a FileLink wears advertises where a click will land. The
 * invariant worth locking down isn't the artwork — it's that the badge and the
 * click can't disagree: both read `defaultOpenTarget`, and `data-open-target`
 * exposes which one the link decided on.
 */

const link = () => screen.getByRole('button');

beforeEach(() => {
  Object.defineProperty(navigator, 'clipboard', {
    value: { writeText: vi.fn().mockResolvedValue(undefined) },
    configurable: true,
  });
});

/** Fire `fn` and collect the bus events it dispatched. */
function captureBuses(fn: () => void): { editor: unknown[]; preview: unknown[] } {
  const editor: unknown[] = [];
  const preview: unknown[] = [];
  const onEditor = (e: Event) => editor.push((e as CustomEvent).detail);
  const onPreview = (e: Event) => preview.push((e as CustomEvent).detail);
  window.addEventListener(EDITOR_OPEN_FILE_EVENT, onEditor);
  window.addEventListener(MARKDOWN_PREVIEW_EVENT, onPreview);
  try {
    fn();
  } finally {
    window.removeEventListener(EDITOR_OPEN_FILE_EVENT, onEditor);
    window.removeEventListener(MARKDOWN_PREVIEW_EVENT, onPreview);
  }
  return { editor, preview };
}

describe('defaultOpenTarget', () => {
  it('sends markdown to the preview and everything else to the editor', () => {
    expect(defaultOpenTarget('/repo/README.md')).toBe('preview');
    expect(defaultOpenTarget('/repo/notes.MARKDOWN')).toBe('preview');
    expect(defaultOpenTarget('/repo/src/app.ts')).toBe('editor');
    expect(defaultOpenTarget('/repo/report.html')).toBe('editor');
    expect(defaultOpenTarget('/repo/Makefile')).toBe('editor');
  });
});

describe('FileLink target badge', () => {
  it('marks a code file as editor-bound and renders one icon', () => {
    const { container } = render(<FileLink path="/repo/src/app.ts" />);
    expect(link()).toHaveAttribute('data-open-target', 'editor');
    expect(container.querySelectorAll('svg')).toHaveLength(1);
  });

  it('marks a markdown file as preview-bound', () => {
    render(<FileLink path="/repo/README.md" />);
    expect(link()).toHaveAttribute('data-open-target', 'preview');
  });

  it('resolves the target from the cwd-joined path, not the raw string', () => {
    // The relative string has the extension, but a caller could pass anything;
    // what matters is the resolved path both the icon and the click use.
    render(<FileLink path="docs/guide.md" cwd="/repo" />);
    expect(link()).toHaveAttribute('data-open-target', 'preview');
    expect(link()).toHaveAttribute('title', expect.stringContaining('/repo/docs/guide.md'));
  });

  it('says the destination in the tooltip', () => {
    render(<FileLink path="/repo/src/app.ts" />);
    expect(link().getAttribute('title')).toContain('opens in the editor');
    render(<FileLink path="/repo/README.md" />);
    expect(screen.getAllByRole('button')[1].getAttribute('title')).toContain(
      'opens a markdown preview',
    );
  });

  it('lets a caller-supplied title win', () => {
    render(<FileLink path="/repo/src/app.ts" title="Bash(cat app.ts)" />);
    expect(link()).toHaveAttribute('title', 'Bash(cat app.ts)');
  });

  it('can be suppressed for hosts that supply their own iconography', () => {
    const { container } = render(<FileLink path="/repo/src/app.ts" icon={false} />);
    expect(container.querySelector('svg')).toBeNull();
    // Still a live link — only the badge is gone.
    expect(link()).toHaveAttribute('data-open-target', 'editor');
  });

  it('badge and click agree: the advertised surface is the one that opens', () => {
    const view = render(<FileLink path="/repo/README.md" />);
    expect(link()).toHaveAttribute('data-open-target', 'preview');
    const md = captureBuses(() => fireEvent.click(link()));
    expect(md.preview).toHaveLength(1);
    expect(md.editor).toHaveLength(0);

    view.unmount();
    render(<FileLink path="/repo/src/app.ts" />);
    expect(link()).toHaveAttribute('data-open-target', 'editor');
    const code = captureBuses(() => fireEvent.click(link()));
    expect(code.editor).toHaveLength(1);
    expect(code.preview).toHaveLength(0);
  });
});

describe('paths detected in assistant prose', () => {
  it('carry the badge too — nothing else marks them as clickable at rest', () => {
    const { container } = render(
      <MarkdownFileCwdProvider value="/repo">
        <Markdown text="I fixed it in `src/lib/chatScroll.ts` — take a look." />
      </MarkdownFileCwdProvider>,
    );
    const path = screen.getByRole('button');
    expect(path).toHaveAttribute('data-open-target', 'editor');
    expect(container.querySelectorAll('svg')).toHaveLength(1);
  });

  it('a markdown mention in prose advertises the preview', () => {
    render(
      <MarkdownFileCwdProvider value="/repo">
        <Markdown text="See `docs/DESIGN.md` for the rationale." />
      </MarkdownFileCwdProvider>,
    );
    expect(screen.getByRole('button')).toHaveAttribute('data-open-target', 'preview');
  });

  it('a relative mention with no cwd stays plain text (no dead badge)', () => {
    const { container } = render(<Markdown text="See `docs/DESIGN.md` for the rationale." />);
    expect(screen.queryByRole('button')).toBeNull();
    expect(container.querySelector('svg')).toBeNull();
  });
});
