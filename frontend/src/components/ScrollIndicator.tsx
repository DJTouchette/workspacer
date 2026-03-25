import React from 'react';
import { PaneConfig } from '../types/pane';

interface ScrollIndicatorProps {
  panes: PaneConfig[];
  activePaneId: string;
  scrollFraction: number; // 0..1
  onDotClick: (id: string) => void;
}

const ScrollIndicator: React.FC<ScrollIndicatorProps> = ({
  panes,
  activePaneId,
  scrollFraction,
  onDotClick,
}) => {
  return (
    <div
      className="scroll-indicator"
      style={{
        position: 'fixed',
        bottom: 0,
        left: 0,
        right: 0,
        height: '8px',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        backgroundColor: 'rgb(20, 20, 24)',
        zIndex: 100,
        gap: '6px',
        padding: '0 16px',
      }}
    >
      {panes.map((pane) => {
        const isActive = pane.id === activePaneId;
        return (
          <div
            key={pane.id}
            onClick={() => onDotClick(pane.id)}
            style={{
              width: isActive ? '16px' : '6px',
              height: '4px',
              borderRadius: '2px',
              backgroundColor: isActive
                ? 'rgb(80, 120, 200)'
                : 'rgb(60, 60, 70)',
              cursor: 'pointer',
              transition: 'none',
            }}
            title={pane.title}
          />
        );
      })}
    </div>
  );
};

export default ScrollIndicator;
