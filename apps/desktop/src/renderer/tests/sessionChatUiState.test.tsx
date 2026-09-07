import React from 'react';
import { act, fireEvent, render, renderHook, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import {
  MAX_RETAINED_CHAT_SESSIONS,
  clearSessionChatUiState,
  retainedChatSessionCount,
  useSessionChatRef,
  useSessionChatState,
} from '../src/hooks/useSessionChatUiState';
import {
  FleetChatDestination,
  RetainedSessionChat,
} from '../src/components/claude/RetainedSessionChat';

describe('retained session chat state', () => {
  it('retains drafts, attachments, pending turns, pagination and refs per session across unmounts', () => {
    const useData = (sid: string) => ({
      draft: useSessionChatState(sid, 'draft', ''),
      files: useSessionChatState<string[]>(sid, 'files', []),
      pending: useSessionChatState<string[]>(sid, 'pending', []),
      page: useSessionChatState(sid, 'page', 60),
      scroll: useSessionChatRef(sid, 'scroll', 0),
    });
    const first = renderHook(() => useData('retained-one'));
    act(() => {
      first.result.current.draft[1]('draft');
      first.result.current.files[1](['image.png']);
      first.result.current.pending[1](['sending']);
      first.result.current.page[1](120);
      first.result.current.scroll.current = 44;
    });
    first.unmount();
    const second = renderHook(() => useData('retained-two'));
    expect(second.result.current.draft[0]).toBe('');
    second.unmount();
    const restored = renderHook(() => useData('retained-one'));
    expect(restored.result.current.draft[0]).toBe('draft');
    expect(restored.result.current.files[0]).toEqual(['image.png']);
    expect(restored.result.current.pending[0]).toEqual(['sending']);
    expect(restored.result.current.page[0]).toBe(120);
    expect(restored.result.current.scroll.current).toBe(44);
    restored.unmount();
    clearSessionChatUiState('retained-one');
    const ended = renderHook(() => useData('retained-one'));
    expect(ended.result.current.draft[0]).toBe('');
  });
  it('delivers a late send result to the remounted card without overwriting a newer draft', () => {
    const useCard = () => ({
      draft: useSessionChatState('late-send', 'draft', ''),
      sending: useSessionChatState('late-send', 'sending', false),
    });
    const first = renderHook(useCard);
    const original = first.result.current;
    act(() => {
      original.draft[1]('submitted');
      original.sending[1](true);
    });
    first.unmount();
    const current = renderHook(useCard);
    act(() => current.result.current.draft[1]('newer draft'));
    act(() => {
      original.draft[1]((text) => (text === 'submitted' ? '' : text));
      original.sending[1](false);
    });
    expect(current.result.current.draft[0]).toBe('newer draft');
    expect(current.result.current.sending[0]).toBe(false);
    act(() => original.draft[1]('failure restored this draft'));
    expect(current.result.current.draft[0]).toBe('failure restored this draft');
  });
  it('bounds retained sessions and fences late writes after cleanup', () => {
    const old = renderHook(() => useSessionChatState('ended-state', 'draft', ''));
    clearSessionChatUiState('ended-state');
    act(() => old.result.current[1]('late'));
    old.unmount();
    const fresh = renderHook(() => useSessionChatState('ended-state', 'draft', ''));
    expect(fresh.result.current[0]).toBe('');
    fresh.unmount();
    for (let i = 0; i < MAX_RETAINED_CHAT_SESSIONS + 10; i++) {
      const h = renderHook(() => useSessionChatState(`bound-${i}`, 'draft', ''));
      h.unmount();
    }
    expect(retainedChatSessionCount()).toBeLessThanOrEqual(MAX_RETAINED_CHAT_SESSIONS);
  });
  it('moves the same DOM and React state between Fleet and the ordinary pane', () => {
    function Content() {
      const [value, set] = React.useState('');
      return (
        <input aria-label="Retained composer" value={value} onChange={(e) => set(e.target.value)} />
      );
    }
    function Fixture({ selected }: { selected: string | null }) {
      return (
        <>
          <RetainedSessionChat sessionId="portal-session" paneId="portal-pane">
            <Content />
          </RetainedSessionChat>
          <FleetChatDestination sessionId={selected} />
        </>
      );
    }
    const view = render(<Fixture selected={null} />);
    const input = screen.getByLabelText('Retained composer');
    fireEvent.change(input, { target: { value: 'my draft' } });
    view.rerender(<Fixture selected="portal-session" />);
    expect(document.querySelector('[data-fleet-chat="portal-session"]')?.contains(input)).toBe(
      true,
    );
    view.rerender(<Fixture selected="another-session" />);
    expect(document.querySelector('[data-fleet-chat="another-session"]')?.contains(input)).toBe(
      false,
    );
    view.rerender(<Fixture selected="portal-session" />);
    expect(screen.getByLabelText('Retained composer')).toBe(input);
    expect(input).toHaveValue('my draft');
  });
});
