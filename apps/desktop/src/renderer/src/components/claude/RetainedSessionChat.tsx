import React, { useLayoutEffect, useRef, useState, useSyncExternalStore } from 'react';
import { createPortal } from 'react-dom';

// Only the selected Fleet destination is registered. The ordinary pane keeps its
// existing controller, subscription and React tree; Fleet never creates a viewer
// for each agent. Moving a stable portal container preserves all descendant state
// (including HTML frames, review cards and composer selection), not just a draft.
type Destination = { sessionId: string | null; paneId?: string; element: HTMLElement | null };
let destination: Destination | null = null;
const listeners = new Set<() => void>();
// Restore before React removes a Fleet destination from the document. Waiting
// for the subscriber render would disconnect frames and reload their documents.
const homes = new Set<{ container: HTMLElement; home: HTMLElement }>();
const subscribe = (fn: () => void) => {
  listeners.add(fn);
  return () => {
    listeners.delete(fn);
  };
};
const snapshot = () => destination;
function publish(next: Destination | null) {
  destination = next;
  listeners.forEach((fn) => fn());
}
export function useFleetChatDestination() {
  return useSyncExternalStore(subscribe, snapshot, snapshot);
}

export function FleetChatDestination({
  sessionId,
  paneId,
}: {
  sessionId: string | null;
  paneId?: string;
}) {
  const ref = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const value = { sessionId, paneId, element: ref.current };
    publish(value);
    return () => {
      for (const owner of homes) {
        if (owner.container.parentElement === value.element) move(owner.container, owner.home);
      }
      if (destination === value) publish(null);
    };
  }, [sessionId, paneId]);
  return (
    <div
      ref={ref}
      data-fleet-chat={sessionId ?? undefined}
      style={{ flex: 1, minHeight: 0, minWidth: 0, position: 'relative' }}
    />
  );
}

function move(element: HTMLElement, parent: HTMLElement) {
  if (element.parentElement === parent) return;
  // State-preserving move also keeps iframe documents alive. appendChild is the
  // older-browser fallback; React state is still retained in either case.
  const target = parent as HTMLElement & { moveBefore?: (node: Node, before: Node | null) => void };
  if (target.moveBefore && element.isConnected && parent.isConnected)
    target.moveBefore(element, null);
  else parent.appendChild(element);
}

export function RetainedSessionChat({
  sessionId,
  paneId,
  children,
}: {
  sessionId: string | null;
  paneId: string;
  children: React.ReactNode;
}) {
  const fleet = useFleetChatDestination();
  const home = useRef<HTMLDivElement>(null);
  const [container] = useState(() => {
    const el = document.createElement('div');
    el.style.cssText = 'height:100%;width:100%;min-height:0;min-width:0';
    return el;
  });
  useLayoutEffect(() => {
    if (!home.current) return;
    const owner = { container, home: home.current };
    homes.add(owner);
    return () => {
      homes.delete(owner);
    };
  }, [container]);
  useLayoutEffect(() => {
    container.dataset.chatSession = sessionId ?? paneId;
    const target =
      sessionId && fleet?.sessionId === sessionId && (!fleet.paneId || fleet.paneId === paneId)
        ? fleet.element
        : home.current;
    if (target) move(container, target);
    return () => {
      if (home.current) move(container, home.current);
    };
  }, [container, fleet, sessionId, paneId]);
  useLayoutEffect(
    () => () => {
      container.remove();
    },
    [container],
  );
  return (
    <>
      <div ref={home} style={{ height: '100%', minHeight: 0 }} />
      {createPortal(children, container)}
    </>
  );
}
