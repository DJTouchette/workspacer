/** Safe recovery copy: provider errors can contain argv, paths or credentials. */
export function spawnFailureMessage(provider: string, _error?: unknown): string {
  const label = provider === 'claude' ? 'Claude Code' : provider === 'codex' ? 'Codex' : provider;
  return `${label} could not start. Check the working directory and provider binary, sign in through the provider CLI if needed, then retry. If it still fails, check Workspacer’s runtime notices.`;
}
