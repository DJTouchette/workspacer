// GitHub Copilot's capability descriptor — the claims here are the ones the
// composer pills and the spawn dialog turn into UI, so each is pinned to what
// was actually verified against the CLI (v1.0.81, 2026-08-28) rather than to
// what its `--help` says. See services/claudemon/src/providers/copilot.rs for
// the capture those verifications came from.

import { describe, it, expect } from 'vitest';
import { capsFor, permissionModeLabel, launchPermissionMode, PROVIDER_CAPS } from './providerCaps';

describe('copilot provider caps', () => {
  it('does not promise approvals it cannot deliver', () => {
    const caps = capsFor('copilot');
    // The IDS stay the shared managed pair — the whole bypass chain (the bus
    // clamp, the brain's launchPermissionMode, the MCP facade) speaks ask/yolo,
    // and a bespoke id here would fail closed somewhere downstream.
    expect(caps.permissionModes.map((m) => m.id)).toEqual(['ask', 'yolo']);
    // …but the LABEL must not say "Ask to approve". Verified: in `copilot -p`
    // mode a blocked tool comes back "Permission denied and could not request
    // permission from user" — the CLI has no channel to ask. What `ask` really
    // buys is path confinement, so that is what the pill says.
    expect(permissionModeLabel('copilot', 'ask')).toBe('Workspace only');
    expect(permissionModeLabel('copilot', 'ask')).not.toMatch(/approve/i);
    expect(permissionModeLabel('copilot', 'yolo')).toBe('Full access');
    // Every other managed provider keeps the approval wording, so this is a
    // copilot-specific correction and not a fleet-wide rename.
    expect(permissionModeLabel('codex', 'ask')).toBe('Ask to approve');
    expect(permissionModeLabel('opencode', 'ask')).toBe('Ask to approve');
  });

  it('still resolves ask/yolo through the shared launch-mode formula', () => {
    // The pill reads launchPermissionMode's output, so copilot has to land on
    // the managed spellings even though its labels differ.
    expect(launchPermissionMode('copilot', true)).toBe('yolo');
    expect(launchPermissionMode('copilot', false)).toBe('ask');
    // And a refused escalation is not echoed back as "full access".
    expect(launchPermissionMode('copilot', false, 'yolo')).toBe('ask');
  });

  it('claims live model + effort switching, which one-shot -p actually gives it', () => {
    const caps = capsFor('copilot');
    // Each turn is a fresh `copilot -p` process, so a switch just changes the
    // next argv — no running process has to accept anything.
    expect(caps.modelSwitch).toBe('live');
    expect(caps.permissionSwitch).toBe('live');
    expect(caps.effort?.switch).toBe('live');
    // Copilot's own seven-level ladder (`copilot --effort` choices).
    expect(caps.effort?.levels.map((l) => l.id)).toEqual([
      'none',
      'minimal',
      'low',
      'medium',
      'high',
      'xhigh',
      'max',
    ]);
  });

  it('is the one managed provider that honestly preserves a conversation across a restart', () => {
    // `copilot --session-id <uuid>` resumes an existing session as readily as it
    // creates one, verified live (a codeword from turn 1 recalled by a second,
    // separately-launched process). Codex/OpenCode/Pi cannot say this.
    expect(capsFor('copilot').restartPreservesConversation).toBe(true);
    expect(PROVIDER_CAPS.codex.restartPreservesConversation).toBe(false);
    expect(PROVIDER_CAPS.opencode.restartPreservesConversation).toBe(false);
    expect(PROVIDER_CAPS.pi.restartPreservesConversation).toBe(false);
  });

  it('takes its model list from the daemon, never from Claude aliases', () => {
    // Copilot's catalog INCLUDES `claude-*` ids, which is exactly the trap:
    // a copilot session must never be fed Claude's alias list (or
    // claude.defaultModel) just because the ids rhyme.
    expect(capsFor('copilot').modelSource).toBe('managed');
  });
});
