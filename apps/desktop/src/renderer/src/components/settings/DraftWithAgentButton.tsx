import React from 'react';
import { IconSpawn } from '../icons';
import { DRAFT_BRIEFS, requestDraftAgent, type DraftBriefId } from '../../lib/draftAgent';

/**
 * The "draft this with an agent" affordance, shared by the Settings sections
 * that offer it.
 *
 * It is a button and a caption, and that is all it is. Everything that decides
 * what happens — the prompt, the tier, the directory, whether the domain has a
 * review step — lives on the brief in lib/draftAgent.ts. This component takes
 * a brief ID and nothing else, so a caller cannot pass it text or ask it for a
 * privilege.
 *
 * Deliberately not a framework. There is no "proposal" abstraction here
 * because only one of the three domains that use it has a propose loop, and a
 * shape guessed from one example is a shape you then have to justify forever.
 */
const DraftWithAgentButton: React.FC<{ briefId: DraftBriefId }> = ({ briefId }) => {
  const brief = DRAFT_BRIEFS[briefId];
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 4, marginTop: 6 }}>
      <button
        onClick={() => requestDraftAgent(brief.id)}
        title={brief.hint}
        style={{
          display: 'inline-flex',
          alignItems: 'center',
          gap: 5,
          alignSelf: 'flex-start',
          padding: '4px 10px',
          fontSize: '0.66rem',
          fontFamily: 'inherit',
          fontWeight: 500,
          background: 'var(--wks-bg-surface)',
          color: 'var(--wks-text-secondary)',
          border: '1px solid var(--wks-border-input)',
          borderRadius: 'var(--wks-radius-pill)',
          cursor: 'pointer',
          transition: 'border-color 0.1s, color 0.1s',
        }}
        onMouseEnter={(e) => {
          (e.currentTarget as HTMLElement).style.borderColor = 'var(--wks-accent)';
          (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-primary)';
        }}
        onMouseLeave={(e) => {
          (e.currentTarget as HTMLElement).style.borderColor = 'var(--wks-border-input)';
          (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-secondary)';
        }}
      >
        <IconSpawn size={12} />
        {brief.label}
      </button>
      <div style={{ fontSize: '0.66rem', color: 'var(--wks-text-faint)', lineHeight: 1.5 }}>
        {brief.hint}
      </div>
    </div>
  );
};

export default DraftWithAgentButton;
