/**
 * FleetDeck vs the command layer (COMMAND_LAYER.md): the deck's window-capture
 * key handler stops propagation on its own keys (y/n/i/hjkl/digits/Enter) and
 * used to have no awareness of the chord dispatcher — both listen on window in
 * the capture phase, so whichever effect re-registered last decided who saw a
 * keystroke first, and prefix chords died whenever an agent card had focus.
 * tmux doctrine: the layer always wins. The deck now yields the leader press
 * itself and EVERY key while the layer is armed (isLayerArmed()); these tests
 * pin that under the worst-case listener order (deck registered first,
 * dispatcher second) and that unarmed keys keep today's deck behavior.
 */
import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, renderHook, cleanup } from '@testing-library/react';
import FleetDeck from '../src/components/FleetDeck';
import { useKeyboardNav } from '../src/hooks/useKeyboardNav';
import { setLayerArmed } from '../src/lib/layerArmed';

const h = vi.hoisted(() => {
  const approve = vi.fn();
  const agent = {
    id: 'a1',
    name: 'alpha',
    sessionId: 's1',
    cwd: '/tmp/alpha',
    global: false,
  };
  const approvalItem = {
    id: 'sig1',
    agentId: 'a1',
    agentName: 'alpha',
    sessionId: 's1',
    kind: 'approval',
    priority: 100,
    createdAt: 0,
    status: 'open',
    title: 'Bash — npm test',
    payload: { type: 'approval', approval: { id: 'ap1' } },
    signature: 'sig1',
  };
  return {
    approve,
    attention: {
      agents: [agent],
      snapshotBySession: {},
      counts: { total: 1, needsYou: 1, byKind: {} },
      setViewLevel: vi.fn(),
      topByAgent: new Map([['a1', approvalItem]]),
      spawnAgent: vi.fn(),
      approve,
      answer: vi.fn(),
      openAgent: vi.fn(),
    },
    approvalItem,
  };
});

// The deck's heavy children aren't under test — a card stub keeps the render
// to the part that owns the key handler.
vi.mock('../src/components/AgentCard', () => ({
  AgentCard: () => <div data-testid="agent-card" />,
}));
vi.mock('../src/contexts/AttentionContext', () => ({
  useAttention: () => h.attention,
}));
vi.mock('../src/hooks/useConfig', () => ({
  useConfig: () => ({
    config: { keybindings: {}, projects: [] },
    saveConfig: vi.fn(),
  }),
}));

const navOptions = (over: Record<string, unknown> = {}) => ({
  tabs: [],
  activeTabId: '',
  activeTab: undefined,
  setActiveTabId: vi.fn(),
  scrollToTab: vi.fn(),
  addTab: vi.fn(() => 't'),
  splitTab: vi.fn(() => 'p'),
  removeTab: vi.fn(),
  removePane: vi.fn(),
  renameTab: vi.fn(),
  moveTab: vi.fn(),
  setActivePane: vi.fn(),
  onToggleHelp: vi.fn(),
  prefix: 'ctrl+space',
  commandLayer: { enabled: true, timeoutMs: 0, repeatMs: 500, passthrough: true },
  ...over,
});

const press = (over: Partial<KeyboardEventInit>): KeyboardEvent => {
  const e = new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...over });
  window.dispatchEvent(e);
  return e;
};

afterEach(() => {
  setLayerArmed(false);
  cleanup();
  vi.clearAllMocks();
});

describe('FleetDeck yields to the command layer', () => {
  it('prefix chord resolves with a card selected, even when the deck handler runs first', () => {
    // Deck mounts (and registers its capture listener) BEFORE the dispatcher —
    // the ordering that used to let the deck eat the chord step.
    render(<FleetDeck top={0} left={0} />);
    const onToggleHelp = vi.fn();
    renderHook(() =>
      useKeyboardNav(navOptions({ onToggleHelp, shortcuts: { 'toggle-help': 'prefix y' } })),
    );

    const leader = press({ key: ' ', code: 'Space', ctrlKey: true }); // arm
    expect(leader.defaultPrevented).toBe(true); // consumed by the DISPATCHER, not dropped
    press({ key: 'y' }); // chord step — collides with the deck's fleet-approve-yes
    expect(onToggleHelp).toHaveBeenCalledTimes(1);
    expect(h.approve).not.toHaveBeenCalled(); // the deck did NOT treat it as an approval
  });

  it('while the layer is armed the deck passes every key through untouched', () => {
    render(<FleetDeck top={0} left={0} />);
    setLayerArmed(true); // no dispatcher mounted — isolates the deck's own guard
    const e = press({ key: 'y' });
    expect(h.approve).not.toHaveBeenCalled();
    expect(e.defaultPrevented).toBe(false); // not consumed: free for the dispatcher
  });

  it('unarmed keys keep the deck behavior: bare y still approves the selected card', () => {
    render(<FleetDeck top={0} left={0} />);
    press({ key: 'y' });
    expect(h.approve).toHaveBeenCalledWith(h.approvalItem, 'yes');
  });

  it('the deck never consumes the leader press itself', () => {
    render(<FleetDeck top={0} left={0} />); // deck listener only, no dispatcher
    const e = press({ key: ' ', code: 'Space', ctrlKey: true });
    expect(e.defaultPrevented).toBe(false);
  });
});
