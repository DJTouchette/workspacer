import React, { useEffect, useRef, useState, useCallback } from 'react';
import type { AgentWorkspace } from '../types/pane';
import type { LibraryItem, LibraryAction } from '../types/library';
import { LIBRARY_RUN_EVENT, dispatchInsert, type LibraryRunDetail } from '../lib/libraryBus';
import { parsePromptVars, gatherAutoContext, applyTemplate, renderItemText, type PromptVar } from '../lib/libraryTemplate';
import PromptVarsDialog from './PromptVarsDialog';

interface Props {
  activeAgent?: AgentWorkspace;
  appCwd: string;
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
  const [pending, setPending] = useState<{ item: LibraryItem; action: LibraryAction; vars: PromptVar[] } | null>(null);

  // Keep latest context in a ref so the (stable) event listener always reads fresh values.
  const ctxRef = useRef({ activeAgent, appCwd });
  useEffect(() => { ctxRef.current = { activeAgent, appCwd }; }, [activeAgent, appCwd]);

  const finalize = useCallback(async (item: LibraryItem, action: LibraryAction, values: Record<string, string>) => {
    const { activeAgent: agent, appCwd: cwd0 } = ctxRef.current;
    const cwd = agent?.cwd || cwd0 || undefined;
    const ctx = await gatherAutoContext({ cwd, sessionId: agent?.sessionId });
    const text = applyTemplate(renderItemText(item), ctx, values);

    if (action === 'copy') {
      try { await navigator.clipboard.writeText(text); } catch { /* ignore */ }
      return;
    }
    if (action === 'spawn') {
      const target = cwd || appCwd;
      if (!target) return;
      recordRecentDir(target);
      spawnAgent({ cwd: target, initialPrompt: text });
      return;
    }
    // insert (default): deliver into the focused agent's pane
    dispatchInsert(text, { sessionId: agent?.sessionId });
  }, [appCwd, spawnAgent, recordRecentDir]);

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
      onSubmit={(values) => { const p = pending; setPending(null); void finalize(p.item, p.action, values); }}
    />
  );
};

export default LibraryHost;
