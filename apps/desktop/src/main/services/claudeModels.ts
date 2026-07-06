/**
 * Single source of truth for the model picker data, shared by the IPC handler
 * (desktop) and the hub capability (web). Aliases resolve to the latest model of
 * each family so they track Claude Code updates with zero maintenance; `seen`
 * carries concrete ids observed in real transcripts — persisted in config plus
 * anything live in the session store.
 */

import { configService } from './configService';
import { claudeSessionStore } from './claudeSessionStore';

export interface ClaudeModelAlias {
  value: string;
  label: string;
  /** One-line purpose blurb rendered under the name in pickers. */
  tagline?: string;
  /** Context-window badge, e.g. '200K' | '1M'. */
  context?: string;
}

export interface ListModelsResult {
  defaultModel: string;
  skipPermissionsDefault: boolean;
  aliases: ClaudeModelAlias[];
  seen: string[];
}

export function listClaudeModels(): ListModelsResult {
  const cfg = configService.getConfig() as any;
  const persisted: string[] = Array.isArray(cfg.claude?.seenModels) ? cfg.claude.seenModels : [];
  const live = claudeSessionStore
    .getAllSnapshots()
    .map((s) => s.usage?.model)
    .filter((m): m is string => !!m);
  const seen = Array.from(new Set([...persisted, ...live])).sort();
  return {
    defaultModel: typeof cfg.claude?.defaultModel === 'string' ? cfg.claude.defaultModel : '',
    skipPermissionsDefault: cfg.claude?.skipPermissionsDefault === true,
    // Each alias tracks the newest model of its family, so these need zero
    // maintenance as Claude Code updates. `sonnet[1m]` is Claude Code's own
    // alias for the million-token-context Sonnet.
    aliases: [
      {
        value: 'fable',
        label: 'Fable',
        tagline: 'Most capable — Mythos-class flagship',
        context: '200K',
      },
      {
        value: 'opus',
        label: 'Opus',
        tagline: 'Deep reasoning for the hardest problems',
        context: '200K',
      },
      {
        value: 'sonnet',
        label: 'Sonnet',
        tagline: 'Fast, balanced daily driver',
        context: '200K',
      },
      {
        value: 'sonnet[1m]',
        label: 'Sonnet 1M',
        tagline: 'Whole codebases in a single context window',
        context: '1M',
      },
      {
        value: 'haiku',
        label: 'Haiku',
        tagline: 'Fastest and lightest, for quick cheap tasks',
        context: '200K',
      },
    ],
    seen,
  };
}
