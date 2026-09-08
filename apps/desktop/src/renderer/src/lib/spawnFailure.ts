/** Safe recovery copy: provider errors can contain argv, paths or credentials. */
export function spawnFailureMessage(provider: string, error?: unknown): string {
  const label = provider === 'claude' ? 'Claude Code' : provider === 'codex' ? 'Codex' : provider;
  const integrationHelp =
    'The selected launch integration could not prepare this session. Check its plugin/service and routing settings, then retry or start a new session with None.';
  const message = error instanceof Error ? error.message : typeof error === 'string' ? error : '';
  // Preserve this safe recovery hint through the manager and dialog wrappers,
  // without displaying a sidecar's raw error (which could contain secrets).
  if (message.includes('[WKS_LAUNCH_INTEGRATION]') || message.includes(integrationHelp)) {
    return `${label} could not start. ${integrationHelp}`;
  }
  return `${label} could not start. Check the working directory and provider binary, sign in through the provider CLI if needed, then retry. If it still fails, check Workspacer’s runtime notices.`;
}
