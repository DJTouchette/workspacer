import React from 'react';

interface AgentPaneProps {
  title: string;
}

const AgentPane: React.FC<AgentPaneProps> = ({ title }) => {
  return (
    <div
      style={{
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        justifyContent: 'center',
        height: '100%',
        backgroundColor: 'rgb(22, 22, 30)',
        color: 'rgb(180, 160, 230)',
        fontSize: '1.2rem',
        gap: '12px',
      }}
    >
      <span style={{ fontSize: '2rem' }}>&#129302;</span>
      <span>{title}</span>
      <span style={{ fontSize: '0.75rem', color: 'rgb(100, 100, 110)' }}>
        Agent placeholder - Phase 2
      </span>
    </div>
  );
};

export default AgentPane;
