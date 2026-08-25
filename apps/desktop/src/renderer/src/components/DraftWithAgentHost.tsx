import React, { useCallback, useEffect, useRef } from 'react';
import type { AgentWorkspace } from '../types/pane';
import { dispatchInsert } from '../lib/libraryBus';
import {
  DRAFT_AGENT_EVENT,
  DRAFT_BRIEFS,
  buildDraftSpawn,
  type DraftAgentDetail,
  type DraftSpawnOptions,
} from '../lib/draftAgent';

interface Props {
  agents: AgentWorkspace[];
  /** Typed to DraftSpawnOptions on purpose: that shape has no
   *  `kickoffMessage`, so this host cannot auto-send even by accident. */
  spawnAgent: (opts: DraftSpawnOptions) => void;
  onSelectAgent: (agentId: string) => void;
}

/**
 * The single executor for "draft this with an agent" (lib/draftAgent.ts).
 * Mounted once in App, beside LibraryHost, because App is where spawn and the
 * agent list live and Settings sections are several layers below it.
 *
 * Notice what this component is NOT given: the focused agent. It resolves the
 * cwd itself from the app's own home, so a draft session cannot inherit
 * whatever repo the user happened to be looking at. That is property 4 in
 * draftAgent.ts, enforced by what is absent from these props.
 */
const DraftWithAgentHost: React.FC<Props> = ({ agents, spawnAgent, onSelectAgent }) => {
  // Latest context in a ref so the mount-once listener always reads fresh
  // values (same shape as LibraryHost).
  const ctxRef = useRef({ agents, spawnAgent, onSelectAgent });
  useEffect(() => {
    ctxRef.current = { agents, spawnAgent, onSelectAgent };
  }, [agents, spawnAgent, onSelectAgent]);

  const run = useCallback(async (detail: DraftAgentDetail) => {
    const brief = DRAFT_BRIEFS[detail.id];
    // The registry IS the allowlist: an id nobody registered launches nothing.
    if (!brief) return;
    const { agents: live, spawnAgent: spawn, onSelectAgent: select } = ctxRef.current;

    // Reuse by fixed name, the Guide/Fleet Manager pattern — pressing the
    // button twice should return you to the conversation, not start a second
    // one. Reuse follows the same never-auto-send rule as a fresh spawn: the
    // text is delivered into the composer for the user to send.
    const running = live.find((a) => !a.global && a.name === brief.agentName && a.sessionId);
    if (running?.sessionId) {
      select(running.id);
      dispatchInsert(brief.prompt, { sessionId: running.sessionId });
      return;
    }

    // The app's own home (~/.workspacer), the same directory the Guide and
    // supervisors open in. These agents are about the app, not about a repo.
    let home = '';
    try {
      home = await window.electronAPI.getSupervisorHome();
    } catch {
      home = '';
    }
    spawn(buildDraftSpawn(brief, home));
  }, []);

  useEffect(() => {
    const handler = (e: Event) => {
      const detail = (e as CustomEvent<DraftAgentDetail>).detail;
      if (!detail?.id) return;
      void run(detail);
    };
    window.addEventListener(DRAFT_AGENT_EVENT, handler);
    return () => window.removeEventListener(DRAFT_AGENT_EVENT, handler);
  }, [run]);

  return null;
};

export default DraftWithAgentHost;
