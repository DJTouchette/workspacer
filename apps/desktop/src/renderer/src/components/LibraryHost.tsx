import React, { useEffect, useRef, useState, useCallback } from 'react';
import type { AgentWorkspace } from '../types/pane';
import type { LibraryItem, LibraryAction } from '../types/library';
import { LIBRARY_RUN_EVENT, dispatchInsert, type LibraryRunDetail } from '../lib/libraryBus';
import {
  parsePromptVars,
  gatherAutoContext,
  applyTemplate,
  renderItemText,
  type PromptVar,
} from '../lib/libraryTemplate';
import PromptVarsDialog from './PromptVarsDialog';

interface Props {
  activeAgent?: AgentWorkspace;
  appCwd: string;
  /**
   * PRE-FILL ONLY. This signature has no `kickoffMessage` (useAgentManager's
   * auto-send field) and must not grow one — see the spawn branch below for
   * why. Widening it here is how the safety property would be lost quietly.
   */
  spawnAgent: (opts: { cwd: string; name?: string; initialPrompt?: string }) => void;
  recordRecentDir: (cwd?: string) => void;
}

/**
 * The single executor for library actions. Mounted once in App, it listens for
 * `library:run`, resolves templating (auto vars + a {{?…}} prompt dialog), then
 * runs the action: insert into the focused agent, spawn a new agent seeded with
 * it, or copy to the clipboard.
 */
const LibraryHost: React.FC<Props> = ({ activeAgent, appCwd, spawnAgent, recordRecentDir }) => {
  const [pending, setPending] = useState<{
    item: LibraryItem;
    action: LibraryAction;
    vars: PromptVar[];
  } | null>(null);

  // Keep latest context in a ref so the (stable) event listener always reads fresh values.
  const ctxRef = useRef({ activeAgent, appCwd });
  useEffect(() => {
    ctxRef.current = { activeAgent, appCwd };
  }, [activeAgent, appCwd]);

  const finalize = useCallback(
    async (item: LibraryItem, action: LibraryAction, values: Record<string, string>) => {
      const { activeAgent: agent, appCwd: cwd0 } = ctxRef.current;
      const cwd = agent?.cwd || cwd0 || undefined;
      const ctx = await gatherAutoContext({ cwd, sessionId: agent?.sessionId });
      const text = applyTemplate(renderItemText(item), ctx, values);

      if (action === 'copy') {
        try {
          await navigator.clipboard.writeText(text);
        } catch {
          /* ignore */
        }
        return;
      }
      if (action === 'spawn') {
        const target = cwd || appCwd;
        if (!target) return;
        recordRecentDir(target);
        // `initialPrompt` PRE-FILLS the composer. The user reads the text and
        // presses Enter. Do NOT change this to `kickoffMessage` (auto-send).
        //
        // This is load-bearing, not a style choice. Library items have a
        // PROJECT scope that lives at `<cwd>/.workspacer/library/*.md`, per
        // repo and committable, and the Library pane and command palette both
        // render a Dispatch button on every item. So a cloned repo can already
        // put a prompt of its choosing one click away. What keeps that safe is
        // exactly this line: the click opens a session with the text sitting
        // in the composer, where a person reads it, rather than running it.
        //
        // The app's own auto-send call sites (spawnGuide, spawnFleetManager)
        // are safe for a reason that does not transfer here: they send text
        // the APP composed in code, around a question the USER typed. Nothing
        // in this path is app-owned — `text` came off disk.
        //
        // Pinned by tests/libraryHostAutoSend.test.tsx.
        spawnAgent({ cwd: target, initialPrompt: text });
        return;
      }
      // insert (default): deliver into the focused agent's pane
      dispatchInsert(text, { sessionId: agent?.sessionId });
    },
    [appCwd, spawnAgent, recordRecentDir],
  );

  useEffect(() => {
    const handler = (e: Event) => {
      const detail = (e as CustomEvent<LibraryRunDetail>).detail;
      if (!detail?.item) return;
      const action: LibraryAction = detail.action || detail.item.action || 'insert';
      const vars = parsePromptVars(renderItemText(detail.item));
      if (vars.length > 0) {
        setPending({ item: detail.item, action, vars });
      } else {
        void finalize(detail.item, action, {});
      }
    };
    window.addEventListener(LIBRARY_RUN_EVENT, handler);
    return () => window.removeEventListener(LIBRARY_RUN_EVENT, handler);
  }, [finalize]);

  if (!pending) return null;
  return (
    <PromptVarsDialog
      title={`Run “${pending.item.title}”`}
      vars={pending.vars}
      onCancel={() => setPending(null)}
      onSubmit={(values) => {
        const p = pending;
        setPending(null);
        void finalize(p.item, p.action, values);
      }}
    />
  );
};

export default LibraryHost;
