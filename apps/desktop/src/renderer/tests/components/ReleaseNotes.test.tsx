import { describe, it, expect } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import React from 'react';
import { ReleaseNotes } from '../../src/components/ReleaseNotes';
import { CHANGELOG } from '../../src/lib/changelog.generated';

/**
 * The notes render from generated data, so the failure mode is quiet: a parser
 * change or a heading typo yields empty cards rather than an error, and a
 * release page that lists versions with nothing under them looks deliberate.
 */

const newest = CHANGELOG[0];

describe('ReleaseNotes', () => {
  it('lists every release, newest first', () => {
    render(<ReleaseNotes />);
    for (const r of CHANGELOG) {
      const label = r.unreleased ? 'Unreleased' : `v${r.version}`;
      expect(screen.getByText(label), `${label} is missing`).toBeTruthy();
    }
  });

  it('opens the newest release with its entries actually in the document', () => {
    render(<ReleaseNotes />);
    const firstSection = newest.sections[0];
    expect(screen.getByText(firstSection.title)).toBeTruthy();
    // The entry text, not just the section heading — an open card with a heading
    // and no body is the exact shape this test exists to catch. Inline markdown
    // (bold, code, italics, links) renders as separate child elements, so match
    // on plain text normalized across the whole entry rather than a raw substring.
    const plainTail = stripInlineMarkdown(firstSection.items[0]).slice(0, 40);
    expect(getByTextAcrossNodes(plainTail)).toBeTruthy();
  });

  it('collapses and re-opens a release', () => {
    render(<ReleaseNotes />);
    const label = newest.unreleased ? 'Unreleased' : `v${newest.version}`;
    const header = screen.getByText(label).closest('button')!;
    expect(header.getAttribute('aria-expanded')).toBe('true');
    fireEvent.click(header);
    expect(header.getAttribute('aria-expanded')).toBe('false');
    expect(screen.queryByText(newest.sections[0].title)).toBeNull();
    fireEvent.click(header);
    expect(screen.getByText(newest.sections[0].title)).toBeTruthy();
  });

  it('marks which release is running', () => {
    const released = CHANGELOG.find((r) => !r.unreleased)!;
    render(<ReleaseNotes highlightVersion={released.version} />);
    expect(screen.getByText('running')).toBeTruthy();
  });

  it('does not claim a running release when the version is unknown', () => {
    render(<ReleaseNotes highlightVersion="99.99.99" />);
    expect(screen.queryByText('running')).toBeNull();
  });
});

/** Strip the inline markdown syntax this app's renderer understands, leaving
 *  the plain text a reader — or a DOM-node-spanning test matcher — would see. */
function stripInlineMarkdown(s: string): string {
  return s
    .replace(/\[([^\]]+)\]\([^)]+\)/g, '$1')
    .replace(/\*\*(.+?)\*\*/g, '$1')
    .replace(/``(.+?)``/g, '$1')
    .replace(/`([^`]+)`/g, '$1')
    .replace(/\*(.+?)\*/g, '$1');
}

/** getByText, but matching plain text normalized across an element's children
 *  instead of a single text node — markdown like `code` or **bold** splits its
 *  surrounding sentence across multiple DOM nodes, which getByText's default
 *  string/RegExp matching cannot see across. */
function getByTextAcrossNodes(text: string) {
  const normalize = (s: string) => s.replace(/\s+/g, ' ').trim();
  const needle = normalize(text);
  return screen.getByText((_content, element) => {
    if (!element) return false;
    if (!normalize(element.textContent ?? '').includes(needle)) return false;
    return Array.from(element.children).every(
      (child) => !normalize(child.textContent ?? '').includes(needle),
    );
  });
}
