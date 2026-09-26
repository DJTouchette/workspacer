import { useCallback, useEffect, useLayoutEffect, useRef, useState, type RefObject } from 'react';
import type { ConversationTurn } from '../types/claudeSession';
import { distanceFromContentEnd } from '../lib/chatScroll';
import {
  firstUnreadTurn,
  loadChatReadingPosition,
  readThrough,
  saveChatReadingPosition,
  type ChatReadingPosition,
} from '../lib/chatReadingPosition';
import { usePageVisible } from './usePageVisible';

export function useChatReadingPosition({
  sessionId,
  active,
  turns,
  offset,
  containerRef,
  stickToBottomRef,
  scrollTopRef,
  tailPadRef,
  visibleCount,
  setVisibleCount,
}: {
  sessionId: string | null;
  active: boolean;
  turns: ConversationTurn[];
  offset: number;
  containerRef: RefObject<HTMLDivElement>;
  stickToBottomRef: { current: boolean };
  scrollTopRef: { current: number };
  tailPadRef: { current: number };
  visibleCount: number;
  setVisibleCount: (count: number) => void;
}) {
  const pageVisible = usePageVisible();
  const [focused, setFocused] = useState(() => document.hasFocus());
  useEffect(() => {
    const focus = () => setFocused(true);
    const blur = () => setFocused(false);
    window.addEventListener('focus', focus);
    window.addEventListener('blur', blur);
    return () => {
      window.removeEventListener('focus', focus);
      window.removeEventListener('blur', blur);
    };
  }, []);
  const watching = active && pageVisible && focused;
  const [unread, setUnread] = useState<number | null>(null);
  const current = useRef<{ key: string; position?: ChatReadingPosition; restored: boolean }>();
  const key = sessionId ? `${location.origin}:${sessionId}` : '';
  const canFollow = useRef(false);
  canFollow.current = watching && !!current.current?.restored;
  const lastSaved = useRef<ChatReadingPosition>();
  const flush = useCallback(() => {
    const value = current.current;
    if (value?.key && value.position && value.position !== lastSaved.current) {
      saveChatReadingPosition(value.key, value.position);
      lastSaved.current = value.position;
    }
  }, []);

  // Run before the pane's follow-tail layout effect. Wait for actual layout and
  // transcript data; an empty attach snapshot must not overwrite a bookmark.
  useLayoutEffect(() => {
    if (current.current?.key !== key) {
      flush();
      current.current = {
        key,
        position: key ? loadChatReadingPosition(key) : undefined,
        restored: false,
      };
      setUnread(null);
    }
    const state = current.current!;
    if (!watching) {
      flush();
      state.restored = false;
      canFollow.current = false;
      return;
    }
    const container = containerRef.current;
    if (state.restored || !container?.clientHeight || !turns.length) return;
    const saved = state.position;
    const nextUnread = firstUnreadTurn(saved?.readThrough, turns, offset);
    const anchorIndex = saved ? Number(saved.anchor.split('-')[1]) : null;
    const targetIndex = anchorIndex !== null && anchorIndex >= offset ? anchorIndex : nextUnread;
    if (targetIndex !== null && targetIndex < offset + turns.length - visibleCount) {
      setVisibleCount(offset + turns.length - targetIndex + 10);
      return;
    }
    if (nextUnread !== unread) {
      setUnread(nextUnread);
      return;
    }
    stickToBottomRef.current = !saved || (saved.atBottom && nextUnread === null);
    if (stickToBottomRef.current) container.scrollTop = container.scrollHeight;
    else {
      const anchor =
        saved &&
        [...container.querySelectorAll<HTMLElement>('[data-chat-anchor]')].find(
          (el) => el.dataset.chatAnchor === saved.anchor,
        );
      if (anchor)
        container.scrollTop +=
          anchor.getBoundingClientRect().top -
          container.getBoundingClientRect().top -
          saved!.offset;
      else container.scrollTop = 0;
    }
    scrollTopRef.current = container.scrollTop;
    state.restored = true;
    canFollow.current = true;
  }, [key, watching, turns, offset, visibleCount, setVisibleCount, flush, unread]);

  useEffect(() => {
    const container = containerRef.current;
    if (!watching || !container || !current.current?.restored) return;
    let frame = 0;
    let timer: ReturnType<typeof setTimeout>;
    const capture = () => {
      const state = current.current;
      if (!state?.restored || !container.clientHeight || !turns.length) return;
      const top = container.getBoundingClientRect().top;
      const anchor = [...container.querySelectorAll<HTMLElement>('[data-chat-anchor]')].find(
        (el) =>
          Number(el.dataset.chatAnchor!.split('-')[1]) < offset + turns.length &&
          el.getBoundingClientRect().bottom > top + 1,
      );
      if (!anchor) return;
      const atBottom =
        distanceFromContentEnd({
          scrollTop: container.scrollTop,
          scrollHeight: container.scrollHeight,
          clientHeight: container.clientHeight,
          tailPad: tailPadRef.current,
        }) <= 24;
      state.position = {
        anchor: anchor.dataset.chatAnchor!,
        offset: anchor.getBoundingClientRect().top - top,
        atBottom,
        readThrough: atBottom ? readThrough(turns, offset) : state.position?.readThrough,
        updatedAt: Date.now(),
      };
      scrollTopRef.current = container.scrollTop;
      clearTimeout(timer);
      timer = setTimeout(flush, 250);
    };
    const schedule = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(capture);
    };
    container.addEventListener('scroll', schedule);
    // Capturing after layout includes streaming growth even when scrollTop stays put.
    schedule();
    return () => {
      container.removeEventListener('scroll', schedule);
      cancelAnimationFrame(frame);
      clearTimeout(timer);
    };
  }, [watching, turns, offset, visibleCount, key, flush, unread]);

  useEffect(() => {
    window.addEventListener('pagehide', flush);
    return () => {
      window.removeEventListener('pagehide', flush);
      flush();
    };
  }, [flush]);

  const jumpToNew = useCallback(() => {
    if (unread === null) return;
    const needed = offset + turns.length - unread + 10;
    if (needed > visibleCount) setVisibleCount(needed);
    requestAnimationFrame(() => {
      const container = containerRef.current;
      const marker = container?.querySelector<HTMLElement>('[data-chat-unread]');
      if (!container || !marker) return;
      stickToBottomRef.current = false;
      container.scrollTop +=
        marker.getBoundingClientRect().top - container.getBoundingClientRect().top - 12;
      scrollTopRef.current = container.scrollTop;
    });
  }, [unread, offset, turns.length, visibleCount, setVisibleCount]);
  return { unread, jumpToNew, clearUnread: () => setUnread(null), canFollow };
}
