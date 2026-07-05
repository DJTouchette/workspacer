import { describe, it, expect } from 'vitest';
import { render } from '@testing-library/react';
import React from 'react';
import { Markdown } from '../../src/components/markdown';

function renderMd(text: string) {
  return render(<Markdown text={text} />);
}

describe('markdown table rendering', () => {
  it('renders a GFM pipe table with header + body cells', () => {
    const { container } = renderMd(
      ['| Tool | Status |', '|------|--------|', '| Read | done |', '| Edit | failed |'].join('\n'),
    );
    const table = container.querySelector('table');
    expect(table).not.toBeNull();
    expect(container.querySelectorAll('thead th')).toHaveLength(2);
    expect(container.querySelectorAll('tbody tr')).toHaveLength(2);
    expect(container.querySelector('thead th')?.textContent).toBe('Tool');
  });

  it('honors column alignment from the delimiter row', () => {
    const { container } = renderMd(['| A | B | C |', '|:--|:-:|--:|', '| 1 | 2 | 3 |'].join('\n'));
    const ths = container.querySelectorAll('thead th');
    expect((ths[0] as HTMLElement).style.textAlign).toBe('left');
    expect((ths[1] as HTMLElement).style.textAlign).toBe('center');
    expect((ths[2] as HTMLElement).style.textAlign).toBe('right');
  });

  it('renders a drawn +---+ ASCII table', () => {
    const { container } = renderMd(
      [
        '+------+--------+',
        '| Tool | Status |',
        '+------+--------+',
        '| Read | done   |',
        '| Edit | failed |',
        '+------+--------+',
      ].join('\n'),
    );
    expect(container.querySelector('table')).not.toBeNull();
    expect(container.querySelectorAll('thead th')).toHaveLength(2);
    expect(container.querySelectorAll('tbody tr')).toHaveLength(2);
  });

  it('renders a Unicode box-drawing table', () => {
    const { container } = renderMd(
      [
        '┌──────┬────────┐',
        '│ Tool │ Status │',
        '├──────┼────────┤',
        '│ Read │ done   │',
        '└──────┴────────┘',
      ].join('\n'),
    );
    expect(container.querySelector('table')).not.toBeNull();
    expect(container.querySelectorAll('thead th')).toHaveLength(2);
    expect(container.querySelectorAll('tbody tr')).toHaveLength(1);
  });

  it('re-renders a drawn table that arrives inside a fenced code block', () => {
    const { container } = renderMd(
      [
        '```',
        '+----+----+',
        '| a  | b  |',
        '+----+----+',
        '| 1  | 2  |',
        '+----+----+',
        '```',
      ].join('\n'),
    );
    expect(container.querySelector('table')).not.toBeNull();
    expect(container.querySelector('pre')).toBeNull();
  });

  it('does NOT treat a paragraph with a stray pipe over --- as a table', () => {
    const { container } = renderMd(['some | text', '---'].join('\n'));
    expect(container.querySelector('table')).toBeNull();
    expect(container.querySelector('hr')).not.toBeNull();
  });

  it('leaves ordinary fenced code as a <pre>', () => {
    const { container } = renderMd(['```js', 'const x = 1;', '```'].join('\n'));
    expect(container.querySelector('pre')).not.toBeNull();
    expect(container.querySelector('table')).toBeNull();
  });
});
