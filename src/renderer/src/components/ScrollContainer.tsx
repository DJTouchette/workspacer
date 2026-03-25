import React, { useRef, useState, useCallback, useEffect, useImperativeHandle, forwardRef } from 'react';
import Pane from './Pane';
import { PaneConfig } from '../types/pane';
import TerminalPane from '../panes/TerminalPane';
import BrowserPane from '../panes/BrowserPane';
import NotesPane from '../panes/NotesPane';
import AgentPane from '../panes/AgentPane';
import { useConfig } from '../hooks/useConfig';

interface ScrollContainerProps {
  panes: PaneConfig[];
  activePaneId: string;
  onPaneFocus: (id: string) => void;
  onPaneClose: (id: string) => void;
  onPaneResize?: (id: string, width: number) => void;
  onPaneResetWidth?: (id: string) => void;
  onPaneMove?: (id: string, toIndex: number) => void;
  onPaneRename?: (id: string, title: string) => void;
  renameSignal?: number;
}

export interface ScrollContainerRef {
  scrollToPane: (id: string) => void;
}

function renderPaneContent(pane: PaneConfig, isActive: boolean) {
  switch (pane.type) {
    case 'terminal':
      return <TerminalPane paneId={pane.id} title={pane.title} isActive={isActive} shell={pane.shell} />;
    case 'browser':
      return <BrowserPane paneId={pane.id} title={pane.title} isActive={isActive} />;
    case 'notes':
      return <NotesPane title={pane.title} />;
    case 'agent':
      return <AgentPane title={pane.title} />;
    default:
      return <div>Unknown pane type</div>;
  }
}

const MIN_PANE_WIDTH = 300;

function ResizeHandle({
  paneId,
  paneWidth,
  onResize,
  onResetWidth,
  containerRef,
}: {
  paneId: string;
  paneWidth: number;
  onResize: (id: string, width: number) => void;
  onResetWidth?: (id: string) => void;
  containerRef: React.RefObject<HTMLDivElement | null>;
}) {
  const handleMouseDown = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      e.stopPropagation();

      const startX = e.clientX;
      const startWidth = paneWidth;
      const maxWidth = containerRef.current
        ? containerRef.current.clientWidth - 100
        : window.innerWidth - 100;

      // Disable scroll-snap during drag
      const container = containerRef.current;
      if (container) container.style.scrollSnapType = 'none';

      document.body.style.cursor = 'col-resize';
      document.body.style.userSelect = 'none';

      const handleMouseMove = (moveEvent: MouseEvent) => {
        const delta = moveEvent.clientX - startX;
        const newWidth = Math.min(maxWidth, Math.max(MIN_PANE_WIDTH, startWidth + delta));
        onResize(paneId, newWidth);
      };

      const handleMouseUp = () => {
        document.body.style.cursor = '';
        document.body.style.userSelect = '';
        if (container) container.style.scrollSnapType = 'x mandatory';
        document.removeEventListener('mousemove', handleMouseMove);
        document.removeEventListener('mouseup', handleMouseUp);
      };

      document.addEventListener('mousemove', handleMouseMove);
      document.addEventListener('mouseup', handleMouseUp);
    },
    [paneId, paneWidth, onResize, containerRef]
  );

  const handleDoubleClick = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      e.stopPropagation();
      onResetWidth?.(paneId);
    },
    [paneId, onResetWidth]
  );

  return (
    <div
      onMouseDown={handleMouseDown}
      onDoubleClick={handleDoubleClick}
      style={{
        width: '8px',
        minWidth: '8px',
        cursor: 'col-resize',
        backgroundColor: 'transparent',
        transition: 'none',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
      }}
      onMouseEnter={(e) => {
        (e.currentTarget as HTMLElement).style.backgroundColor = 'rgb(55, 55, 65)';
      }}
      onMouseLeave={(e) => {
        (e.currentTarget as HTMLElement).style.backgroundColor = 'transparent';
      }}
    >
      <div style={{
        width: '2px',
        height: '40px',
        borderRadius: '1px',
        backgroundColor: 'inherit',
      }} />
    </div>
  );
}

const ScrollContainer = forwardRef<ScrollContainerRef, ScrollContainerProps>(
  ({ panes, activePaneId, onPaneFocus, onPaneClose, onPaneResize, onPaneResetWidth, onPaneMove, onPaneRename, renameSignal }, ref) => {
    const containerRef = useRef<HTMLDivElement>(null);
    const { config } = useConfig();
    const peek = config.panes.peek ?? 80;
    const gap = config.panes.gap ?? 16;

    // Compute pane width dynamically: viewport - 2 * peek - 2 * gap(margin)
    const [paneWidth, setPaneWidth] = useState(800);

    useEffect(() => {
      const container = containerRef.current;
      if (!container) return;

      const updateWidth = () => {
        const w = container.clientWidth - 2 * peek - gap;
        setPaneWidth(Math.max(400, w));
      };

      updateWidth();
      const observer = new ResizeObserver(updateWidth);
      observer.observe(container);
      return () => observer.disconnect();
    }, [peek, gap]);

    const scrollToPane = useCallback((id: string) => {
      const container = containerRef.current;
      if (!container) return;
      const paneEl = container.querySelector(`[data-pane-id="${id}"]`) as HTMLElement | null;
      if (!paneEl) return;

      const containerRect = container.getBoundingClientRect();
      const paneRect = paneEl.getBoundingClientRect();
      const scrollLeft =
        paneEl.offsetLeft - containerRect.width / 2 + paneRect.width / 2;

      container.scrollTo({
        left: scrollLeft,
        behavior: 'instant',
      });
    }, []);

    useImperativeHandle(ref, () => ({
      scrollToPane,
    }), [scrollToPane]);

    // Detect which pane is most visible after scroll ends
    useEffect(() => {
      const container = containerRef.current;
      if (!container) return;

      let scrollTimeout: ReturnType<typeof setTimeout>;

      const handleScrollEnd = () => {
        clearTimeout(scrollTimeout);
        scrollTimeout = setTimeout(() => {
          const containerCenter = container.scrollLeft + container.clientWidth / 2;
          let closestId = panes[0]?.id;
          let closestDist = Infinity;

          for (const pane of panes) {
            const el = container.querySelector(`[data-pane-id="${pane.id}"]`) as HTMLElement | null;
            if (!el) continue;
            const paneCenter = el.offsetLeft + el.offsetWidth / 2;
            const dist = Math.abs(containerCenter - paneCenter);
            if (dist < closestDist) {
              closestDist = dist;
              closestId = pane.id;
            }
          }

          if (closestId && closestId !== activePaneId) {
            onPaneFocus(closestId);
          }
        }, 100);
      };

      container.addEventListener('scroll', handleScrollEnd, { passive: true });
      return () => {
        container.removeEventListener('scroll', handleScrollEnd);
        clearTimeout(scrollTimeout);
      };
    }, [panes, activePaneId, onPaneFocus]);

    const handlePaneMove = useCallback((id: string, delta: number) => {
      if (!onPaneMove) return;
      const idx = panes.findIndex((p) => p.id === id);
      if (idx < 0) return;
      onPaneMove(id, idx + delta);
    }, [panes, onPaneMove]);

    return (
      <div
        ref={containerRef}
        className="scroll-container"
        style={{
          display: 'flex',
          flexDirection: 'row',
          overflowX: 'auto',
          overflowY: 'hidden',
          height: '100%',
          scrollSnapType: 'x mandatory',
          scrollBehavior: 'auto',
          padding: '0',
          gap: '0px',
          alignItems: 'stretch',
        }}
      >
        {panes.map((pane) => (
          <div
            key={pane.id}
            style={{
              scrollSnapAlign: 'center',
              flexShrink: 0,
              height: '100%',
              display: 'flex',
              alignItems: 'stretch',
            }}
          >
            <Pane
              id={pane.id}
              type={pane.type}
              title={pane.title}
              width={pane.widthOverride ?? paneWidth}
              isActive={pane.id === activePaneId}
              onClose={onPaneClose}
              onFocus={onPaneFocus}
              onMove={onPaneMove ? handlePaneMove : undefined}
              onRename={onPaneRename}
              renameSignal={pane.id === activePaneId ? renameSignal : undefined}
            >
              {renderPaneContent(pane, pane.id === activePaneId)}
            </Pane>
            {/* Resize handle in the gap between panes */}
            {onPaneResize && (
              <ResizeHandle
                paneId={pane.id}
                paneWidth={pane.widthOverride ?? paneWidth}
                onResize={onPaneResize}
                onResetWidth={onPaneResetWidth}
                containerRef={containerRef}
              />
            )}
          </div>
        ))}
      </div>
    );
  }
);

ScrollContainer.displayName = 'ScrollContainer';

export default ScrollContainer;
