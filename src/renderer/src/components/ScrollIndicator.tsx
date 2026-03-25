import React from 'react';
import { TabConfig } from '../types/pane';

interface ScrollIndicatorProps {
  tabs: TabConfig[];
  activeTabId: string;
  onDotClick: (id: string) => void;
}

const ScrollIndicator: React.FC<ScrollIndicatorProps> = ({
  tabs,
  activeTabId,
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
      {tabs.map((tab) => {
        const isActive = tab.id === activeTabId;
        return (
          <div
            key={tab.id}
            onClick={() => onDotClick(tab.id)}
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
            title={tab.title}
          />
        );
      })}
    </div>
  );
};

export default ScrollIndicator;
