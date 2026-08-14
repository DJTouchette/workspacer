/**
 * The at-a-glance project mark. The derivation itself is covered in
 * lib/projectIdentity.test.ts; this pins what actually gets DRAWN — in
 * particular the fallback chain, since a broken icon host must never cost you
 * the ability to tell your projects apart.
 */
import { describe, it, expect } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import React from 'react';
import { ProjectMark } from '../../src/components/ProjectMark';

describe('ProjectMark', () => {
  it('draws derived initials with no configuration at all', () => {
    const { container } = render(<ProjectMark cwd="/work/api-gateway" />);
    expect(container.textContent).toBe('AG');
    // A colour is always resolved, so the plate is never invisible.
    expect(container.querySelector('span[style*="color"]')).toBeTruthy();
  });

  it('prefers a configured icon over initials', () => {
    const { container } = render(
      <ProjectMark cwd="/work/repo" projects={{ '/work/repo': { icon: '🚀' } }} />,
    );
    expect(container.textContent).toBe('🚀');
  });

  it('prefers a favicon over both, and labels it for the project', () => {
    const { container } = render(
      <ProjectMark
        cwd="/work/repo"
        projects={{ '/work/repo': { icon: '🚀', favicon: 'https://x/icon.png' } }}
      />,
    );
    const img = container.querySelector('img');
    expect(img?.getAttribute('src')).toBe('https://x/icon.png');
    expect(container.textContent).toBe('');
  });

  it('falls back to the derived mark when the favicon fails to load', () => {
    // An icon host being down must not leave a hole in every card.
    const { container } = render(
      <ProjectMark
        cwd="/work/api-gateway"
        projects={{ '/work/api-gateway': { favicon: 'https://x/404.png' } }}
      />,
    );
    const img = container.querySelector('img')!;
    expect(img).toBeTruthy();
    fireEvent.error(img); // via fireEvent so React's state update is flushed
    expect(container.querySelector('img')).toBeNull();
    expect(container.textContent).toBe('AG');
  });

  it('names the project in its tooltip, so the mark is never a mystery', () => {
    render(<ProjectMark cwd="/work/repo" projects={{ '/work/repo': { label: 'Platform' } }} />);
    expect(screen.getByTitle('Platform')).toBeTruthy();
  });

  it('renders the label beside the mark when asked', () => {
    const { container } = render(<ProjectMark cwd="/work/repo" withLabel />);
    expect(container.textContent).toContain('repo');
  });

  it('renders nothing without a directory — the Overview card has no project', () => {
    const { container } = render(<ProjectMark cwd="" />);
    expect(container.innerHTML).toBe('');
  });
});
