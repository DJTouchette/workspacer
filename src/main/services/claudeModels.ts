/**
 * Single source of truth for the model picker data, shared by the IPC handler
 * (desktop) and the hub capability (web). Aliases resolve to the latest model of
 * each family so they track Claude Code updates with zero maintenance; `seen`
 * carries concrete ids observed in real transcripts — persisted in config plus
 * anything live in the session store.
 */

import { configService } from './configService';
import { claudeSessionStore } from './claudeSessionStore';

export interface ListModelsResult {
  defaultModel: string;
  skipPermissionsDefault: boolean;
  aliases: Array<{ value: string; label: string }>;
  seen: string[];
}

export function listClaudeModels(): ListModelsResult {
  const cfg = configService.getConfig() as any;
  const persisted: string[] = Array.isArray(cfg.claude?.seenModels) ? cfg.claude.seenModels : [];
  const live = claudeSessionStore.getAllSnapshots()
    .map((s) => s.usage?.model)
    .filter((m): m is string => !!m);
  const seen = Array.from(new Set([...persisted, ...live])).sort();
  return {
    defaultModel: typeof cfg.claude?.defaultModel === 'string' ? cfg.claude.defaultModel : '',
    skipPermissionsDefault: cfg.claude?.skipPermissionsDefault === true,
    aliases: [
      { value: 'opus', label: 'Opus — latest' },
      { value: 'sonnet', label: 'Sonnet — latest' },
      { value: 'haiku', label: 'Haiku — latest' },
    ],
    seen,
  };
}
