import React from 'react';

interface BrowserPaneProps {
  title: string;
}

const BrowserPane: React.FC<BrowserPaneProps> = ({ title }) => {
  return (
    <div
      style={{
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        justifyContent: 'center',
        height: '100%',
        backgroundColor: 'rgb(24, 24, 30)',
        color: 'rgb(160, 180, 230)',
        fontSize: '1.2rem',
        gap: '12px',
      }}
    >
      <span style={{ fontSize: '2rem' }}>&#127760;</span>
      <span>{title}</span>
      <span style={{ fontSize: '0.75rem', color: 'rgb(100, 100, 110)' }}>
        Browser placeholder - Phase 2
      </span>
    </div>
  );
};

export default BrowserPane;
