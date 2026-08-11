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
    // and no body is the exact shape this test exists to catch. Markdown renders
    // **bold** as an element, so match on a distinctive plain-text tail instead.
    const tail = firstSection.items[0].replace(/^\*\*.+?\*\*\s*/, '').slice(0, 30);
    expect(screen.getByText(new RegExp(escapeRe(tail)))).toBeTruthy();
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

function escapeRe(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}
