/**
 * Regression test: keyboard navigation order must match the visual row order.
 *
 * The palette renders groups as actions → apps → commands → … but the unified
 * item array (which ArrowUp/Down step through, and which each row highlights by
 * its index in) had commands BEFORE apps. So when both an app and a command
 * were present, ArrowDown jumped past the app to the last row and then back up —
 * the highlight bounced instead of advancing one visual row at a time.
 */
import { describe, it, expect, vi } from 'vitest';
import { render, fireEvent, screen } from '@testing-library/react';
import CommandPalette from '../src/components/CommandPalette';
import { ConfigProvider } from '../src/contexts/ConfigContext';

// Rows and the highlight are found by data attributes, not by their styling:
// probing exact CSS values made this test fail on every palette restyle while
// the behaviour it guards was fine.
function paletteRows(container: HTMLElement): HTMLElement[] {
  return Array.from(container.querySelectorAll('[data-palette-row]')) as HTMLElement[];
}

const highlightedIndex = (rows: HTMLElement[]) =>
  rows.findIndex((r) => r.dataset.selected === 'true');

describe('CommandPalette — keyboard nav order matches visual order', () => {
  it('ArrowDown advances the highlight one visual row at a time', () => {
    const { container } = render(
      // The palette reads the UI mode from config (useUiMode), so it needs the
      // ConfigProvider around it like in the app.
      <ConfigProvider>
        <CommandPalette
          visible
          apps={[{ name: 'MyApp', url: 'https://example.com' }]}
          onClose={vi.fn()}
          onLaunchApp={vi.fn()}
          onAddTab={vi.fn()}
          onSpawnAgent={vi.fn()} // surfaces a "Spawn Agent" command row
        />
      </ConfigProvider>,
    );

    const input = container.querySelector('input')!;
    const rowCount = paletteRows(container).length;
    // Sanity: both an app and at least one command row are present (the
    // visual-vs-nav-order bug needs a mixed list). Kept as a robust lower bound
    // rather than an exact count so it doesn't break when commands are added/removed.
    expect(rowCount).toBeGreaterThanOrEqual(2);

    // selectedIndex starts at 0; after k ArrowDowns it is k. The highlighted
    // row's position in the DOM must equal k for every step — i.e. the visual
    // order and the navigation order are the same list.
    for (let k = 0; k < rowCount; k++) {
      const rows = paletteRows(container);
      expect(highlightedIndex(rows)).toBe(k);
      fireEvent.keyDown(input, { key: 'ArrowDown' });
    }
  });

  it('surfaces Ask the Fleet as the only primary supervisor command', () => {
    render(
      <ConfigProvider>
        <CommandPalette
          visible
          apps={[]}
          onClose={vi.fn()}
          onLaunchApp={vi.fn()}
          onAddTab={vi.fn()}
          onOpenAskPane={vi.fn()}
        />
      </ConfigProvider>,
    );

    expect(screen.getByText('Ask the Fleet')).toBeInTheDocument();
    expect(screen.queryByText('Spawn Fleet Agent')).not.toBeInTheDocument();
  });

  it('only shows the agent monitor command when supplied by the app', () => {
    const { rerender } = render(
      <ConfigProvider>
        <CommandPalette visible apps={[]} onClose={vi.fn()} onLaunchApp={vi.fn()} />
      </ConfigProvider>,
    );

    expect(screen.queryByText('Open Agent Monitor')).not.toBeInTheDocument();

    rerender(
      <ConfigProvider>
        <CommandPalette
          visible
          apps={[]}
          onClose={vi.fn()}
          onLaunchApp={vi.fn()}
          onOpenAgents={vi.fn()}
        />
      </ConfigProvider>,
    );

    expect(screen.getByText('Open Agent Monitor')).toBeInTheDocument();
  });
});

/**
 * Typing something that isn't a command is a task, not a typo: the palette
 * offers to hand it to a fresh agent. The risk this guards is the offer being
 * too eager — it must never take Enter away from a real command match.
 */
describe('CommandPalette — spawn an agent on the typed text', () => {
  const open = (onSpawnAgent: ReturnType<typeof vi.fn>) =>
    render(
      <ConfigProvider>
        <CommandPalette
          visible
          apps={[]}
          onClose={vi.fn()}
          onLaunchApp={vi.fn()}
          onAddTab={vi.fn()}
          onSpawnAgent={onSpawnAgent}
        />
      </ConfigProvider>,
    );

  it('offers no spawn row until something is typed', () => {
    const { container } = open(vi.fn());
    expect(container.textContent).not.toContain('Hand it to an agent');
  });

  it('Enter on free text spawns immediately with it as the first message', () => {
    const onSpawnAgent = vi.fn();
    const { container } = open(onSpawnAgent);
    const input = container.querySelector('input')!;

    fireEvent.change(input, { target: { value: 'fix the login redirect' } });
    // Nothing else matches, so the spawn row is the only — and selected — row.
    expect(screen.getByText('New agent: fix the login redirect')).toBeInTheDocument();
    fireEvent.keyDown(input, { key: 'Enter' });

    expect(onSpawnAgent).toHaveBeenCalledWith({ prompt: 'fix the login redirect' });
  });

  it('⌘/Ctrl+Enter carries the text into the full spawn dialog instead', () => {
    const onSpawnAgent = vi.fn();
    const { container } = open(onSpawnAgent);
    const input = container.querySelector('input')!;

    fireEvent.change(input, { target: { value: 'audit the auth flow' } });
    fireEvent.keyDown(input, { key: 'Enter', ctrlKey: true });

    expect(onSpawnAgent).toHaveBeenCalledWith({
      prompt: 'audit the auth flow',
      openDialog: true,
    });
  });

  it('never steals Enter from a real match — the spawn row sorts last', () => {
    const onAddTab = vi.fn();
    const onSpawnAgent = vi.fn();
    const { container } = render(
      <ConfigProvider>
        <CommandPalette
          visible
          apps={[]}
          onClose={vi.fn()}
          onLaunchApp={vi.fn()}
          onAddTab={onAddTab}
          onSpawnAgent={onSpawnAgent}
        />
      </ConfigProvider>,
    );
    const input = container.querySelector('input')!;

    fireEvent.change(input, { target: { value: 'terminal' } });
    fireEvent.keyDown(input, { key: 'Enter' });

    expect(onAddTab).toHaveBeenCalled();
    expect(onSpawnAgent).not.toHaveBeenCalled();
  });

  it('finds the spawn action by the name the sidebar button uses', () => {
    const { container } = open(vi.fn());
    const input = container.querySelector('input')!;
    fireEvent.change(input, { target: { value: 'new agent' } });
    // Matched via keywords — the row itself is still labelled "New Claude Code".
    expect(screen.getByText('New Claude Code')).toBeInTheDocument();
  });
});

describe('CommandPalette — window-level keyboard net', () => {
  // If the input's focus claim loses (a webview guest refusing to release
  // focus — the Windows "Ctrl+Shift+P kills my keyboard" report), the palette
  // must still be fully drivable from window-level keydowns.
  it('Escape dismisses even when the input never got focus', () => {
    const onClose = vi.fn();
    const { container } = render(
      <ConfigProvider>
        <CommandPalette
          visible
          apps={[]}
          onClose={onClose}
          onLaunchApp={vi.fn()}
          onAddTab={vi.fn()}
        />
      </ConfigProvider>,
    );
    (container.querySelector('input') as HTMLInputElement).blur();
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(onClose).toHaveBeenCalled();
  });

  it('a stray printable keystroke seeds the query and reclaims the input', () => {
    const { container } = render(
      <ConfigProvider>
        <CommandPalette
          visible
          apps={[]}
          onClose={vi.fn()}
          onLaunchApp={vi.fn()}
          onAddTab={vi.fn()}
        />
      </ConfigProvider>,
    );
    const input = container.querySelector('input') as HTMLInputElement;
    input.blur();
    fireEvent.keyDown(window, { key: 'x' });
    expect(input.value).toBe('x');
    expect(document.activeElement).toBe(input);
  });
});

/**
 * The palette is the app's front door — it opens on ⌘/Ctrl+K, and it tells you
 * how to drive it. Both are easy to lose in a restyle, so they're pinned here.
 */
describe('CommandPalette — the bar itself', () => {
  const open = () =>
    render(
      <ConfigProvider>
        <CommandPalette
          visible
          apps={[{ name: 'MyApp', url: 'https://example.com' }]}
          onClose={vi.fn()}
          onOpenPane={vi.fn()}
          onOpenApp={vi.fn()}
        />
      </ConfigProvider>,
    );

  it('shows how to navigate and how many results it found', () => {
    const { container } = open();
    expect(screen.getByText('navigate')).toBeInTheDocument();
    expect(screen.getByText('open')).toBeInTheDocument();
    // The count is asserted by shape, not by an exact number: the command list
    // grows with the app, and pinning a literal here would make every new
    // palette entry a failing test.
    const counts = Array.from(container.querySelectorAll('span'))
      .map((el) => (el.textContent ?? '').trim())
      .filter((t) => /^\d+ results?$/.test(t));
    expect(counts).toHaveLength(1);
    expect(parseInt(counts[0], 10)).toBeGreaterThan(0);
  });

  it('offers escape as a visible way out, not just a key that happens to work', () => {
    open();
    expect(screen.getByText('esc')).toBeInTheDocument();
  });
});

describe('default keybindings', () => {
  it('opens the palette on mod+K', async () => {
    const { DEFAULT_CONFIG } = await import('../src/hooks/configDefaults');
    const { KEYBINDING_PRESETS } = await import('../src/lib/keybindingPresets');
    // The shipped config and the default preset must agree, or a fresh install
    // and a preset re-pick would bind different keys.
    expect(DEFAULT_CONFIG.keybindings.shortcuts['command-palette']).toBe('mod+k');
    const { DEFAULT_PRESET_ID } = await import('../src/lib/keybindingPresets');
    expect(KEYBINDING_PRESETS[DEFAULT_PRESET_ID].shortcuts['command-palette']).toBe('mod+k');
  });
});
