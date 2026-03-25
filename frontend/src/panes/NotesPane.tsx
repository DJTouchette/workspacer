import React from 'react';

interface NotesPaneProps {
  title: string;
}

const NotesPane: React.FC<NotesPaneProps> = ({ title }) => {
  return (
    <div
      style={{
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        justifyContent: 'center',
        height: '100%',
        backgroundColor: 'rgb(26, 26, 30)',
        color: 'rgb(230, 210, 160)',
        fontSize: '1.2rem',
        gap: '12px',
      }}
    >
      <span style={{ fontSize: '2rem' }}>&#128221;</span>
      <span>{title}</span>
      <span style={{ fontSize: '0.75rem', color: 'rgb(100, 100, 110)' }}>
        Notes placeholder - Phase 2
      </span>
    </div>
  );
};

export default NotesPane;
