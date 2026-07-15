// Maps a live agent ambient state to the Workspacer pack's status glyph, for
// use beside a status *label* (the dots stay dots — these detailed glyphs read
// best at label size, not as 10px corner badges). Returns null for states with
// no pack equivalent (e.g. stopped), so callers fall back to label-only.
import React from 'react';
import type { SessionAmbientState } from '../types/claudeSession';
import { IconWorking, IconIdle, IconReviewing, IconQueued, type WksIconProps } from './wksIcons';

export const StatusGlyph: React.FC<{ state: SessionAmbientState | undefined } & WksIconProps> = ({
  state,
  ...rest
}) => {
  switch (state) {
    case 'thinking':
    case 'streaming':
    case 'background':
      return <IconWorking {...rest} />;
    case 'waiting_approval':
      return <IconReviewing {...rest} />;
    case 'waiting_input':
      return <IconQueued {...rest} />;
    case 'idle':
      return <IconIdle {...rest} />;
    default:
      return null;
  }
};
