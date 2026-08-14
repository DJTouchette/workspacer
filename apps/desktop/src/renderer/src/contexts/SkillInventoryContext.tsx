import React, { createContext, useContext, useMemo } from 'react';
import type { ContextItemInfo } from '../types/claudeSession';

/**
 * The session's skill inventory, by skill name.
 *
 * A `Skill` tool call carries only the skill's NAME — the chat UI rendered it
 * as an anonymous "other" tool row, which is the least informative thing on
 * screen for the one tool call that changes how the agent behaves for the rest
 * of the turn. The session's stream `init` frame already itemizes every skill
 * (claudemon resolves each one's file, origin and description), so the card
 * looks the invoked name up here rather than inventing a second source.
 *
 * Empty outside a Claude stream session — PTY and non-Claude sessions report no
 * inventory, and the card degrades to the bare name.
 */
const SkillInventoryContext = createContext<Map<string, ContextItemInfo>>(new Map());

export const SkillInventoryProvider: React.FC<{
  skills?: ContextItemInfo[];
  children: React.ReactNode;
}> = ({ skills, children }) => {
  const byName = useMemo(() => {
    const m = new Map<string, ContextItemInfo>();
    for (const s of skills ?? []) m.set(s.name.toLowerCase(), s);
    return m;
  }, [skills]);
  return <SkillInventoryContext.Provider value={byName}>{children}</SkillInventoryContext.Provider>;
};

/** What the session knows about the named skill, if anything. */
export function useSkillInfo(name?: string): ContextItemInfo | undefined {
  const byName = useContext(SkillInventoryContext);
  return name ? byName.get(name.toLowerCase()) : undefined;
}
