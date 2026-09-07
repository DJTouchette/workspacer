import React, { useState } from 'react';
import { SmallButton } from './settings/primitives';
import { openPolicySettings } from '../lib/settingsBus';
const DISMISSED = 'wks:first-task-guidance-dismissed';

/** Conversation UI only; does not interpret completion or persist manager graphs. */
export function FirstTaskGuidance({ inFleet }: { inFleet: boolean }) {
  const [dismissed, setDismissed] = useState(() => {
    try {
      return localStorage.getItem(DISMISSED) === 'true';
    } catch {
      return false;
    }
  });
  if (dismissed) return null;
  return (
    <aside
      aria-label="First task guidance"
      style={{ padding: '8px 12px', fontSize: '0.72rem', color: 'var(--wks-text-secondary)' }}
    >
      {inFleet
        ? 'Follow this chat for replies and results. Needs-attention items appear in Inbox. Back to fleet returns to the agents and their summaries.'
        : 'Follow this chat for replies and results. Approval requests and questions appear here and in Inbox. Fleet gives an overview of your agents.'}{' '}
      Stopped sessions offer Resume to continue the conversation.
      <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap', marginTop: 8 }}>
        <SmallButton label="Model routing" onClick={() => openPolicySettings('routing')} />
        <SmallButton label="Fleet workflows" onClick={() => openPolicySettings('supervisor')} />
        <SmallButton
          label="Dismiss task guidance"
          onClick={() => {
            setDismissed(true);
            try {
              localStorage.setItem(DISMISSED, 'true');
            } catch {
              /* local dismissal still works */
            }
          }}
        />
      </div>
    </aside>
  );
}
