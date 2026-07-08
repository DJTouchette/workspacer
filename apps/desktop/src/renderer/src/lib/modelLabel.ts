/** Compact, provider-agnostic model label for display. Strips a leading
 *  "provider/" segment (OpenCode-style ids like `anthropic/claude-sonnet-4`),
 *  the `claude-` vendor prefix, and a trailing date/build stamp.
 *
 *  Examples:
 *    "claude-opus-4-8-20250101"        → "opus-4-8"
 *    "anthropic/claude-sonnet-4"       → "sonnet-4"
 *    "openai/gpt-5.4"                  → "gpt-5.4"
 *    "gpt-5.4"                         → "gpt-5.4"
 */
export function shortModelLabel(model?: string): string {
  // Guard the type, not just falsiness: a model value deserialized from the hub
  // bus (web/remote) isn't guaranteed to be a string, and calling `.replace` on
  // a non-string would throw inside a render and blank the pane.
  if (!model || typeof model !== 'string') return '';
  return model
    .replace(/^[\w.-]+\//, '') // drop a "provider/" prefix (opencode)
    .replace(/^claude-/, '') // drop the claude vendor prefix
    .replace(/-\d{6,}$/, ''); // drop a trailing date/build stamp
}
