/**
 * Agent-authored handoff brief — the "rich" tier of the cross-provider
 * handoff. Instead of the daemon's mechanical digest, we ask the SOURCE agent
 * to write the brief itself: it's the only thing that has the session in
 * context (why things were done, dead ends, constraints that never made it
 * into the transcript text), so its brief beats anything derived from the
 * conversation log.
 *
 * Flow: pick a brief path under ~/.workspacer/handoffs/, send the source one
 * instruction message through the normal message pipeline (settle+verify,
 * queues if the agent is mid-turn), then poll for the file to appear. If the
 * agent doesn't deliver within the window (dead session, refused, stuck on a
 * permission prompt), fall back to the daemon's deterministic brief so the
 * handoff still happens.
 */

import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { claudemonSessionClient } from './claudemonSessionClient';

/** How long the source agent gets to write the file. Generous: the request
 *  may queue behind an in-flight turn before its own turn even starts. */
const AGENT_BRIEF_TIMEOUT_MS = 150_000;
const POLL_MS = 1_000;

export interface HandoffBriefResult {
  ok: boolean;
  path?: string;
  /** True when the agent didn't deliver and the mechanical brief was used. */
  fallback?: boolean;
  error?: string;
}

function briefInstruction(briefPath: string): string {
  return (
    `Stop what you're doing and write a handoff brief to ${briefPath} — another AI coding agent ` +
    `is about to take over this session and will read that file first. Create the file (markdown) with:\n` +
    `1. The goal of this session, in one paragraph.\n` +
    `2. State of the work: what's done and verified, what's in progress, what hasn't been started.\n` +
    `3. Key files touched and why.\n` +
    `4. Decisions and constraints your successor must respect (including approaches tried and rejected, and why).\n` +
    `5. Gotchas or surprises you hit.\n` +
    `6. The exact next step you would take.\n` +
    `Write only that file, then reply "Handoff brief written." — do not continue any other work.`
  );
}

async function fallbackMechanical(sessionId: string, reason: string): Promise<HandoffBriefResult> {
  const det = await claudemonSessionClient.handoffBrief(sessionId);
  if (det.ok && det.path) return { ok: true, path: det.path, fallback: true, error: reason };
  return {
    ok: false,
    error: `${reason}; mechanical fallback also failed: ${det.error ?? 'unknown'}`,
  };
}

/**
 * Ask the source agent to author the handoff brief; resolve once the file
 * exists (or with the mechanical fallback). The returned path is always safe
 * to hand to the successor when `ok` is true.
 */
export async function agentHandoffBrief(sessionId: string): Promise<HandoffBriefResult> {
  const dir = path.join(os.homedir(), '.workspacer', 'handoffs');
  await fs.promises.mkdir(dir, { recursive: true });
  const ts = new Date().toISOString().replace(/[-:]/g, '').replace('T', '-').slice(0, 15);
  const briefPath = path.join(dir, `${ts}-${sessionId.slice(0, 8)}-agent.md`);

  try {
    const sent = await claudemonSessionClient.message(sessionId, briefInstruction(briefPath));
    if (!sent.ok) {
      return fallbackMechanical(
        sessionId,
        `session can't take a message right now (mode: ${sent.mode ?? 'unknown'})`,
      );
    }
  } catch (err) {
    return fallbackMechanical(
      sessionId,
      `could not reach the source session (${err instanceof Error ? err.message : String(err)})`,
    );
  }

  const deadline = Date.now() + AGENT_BRIEF_TIMEOUT_MS;
  while (Date.now() < deadline) {
    try {
      const st = await fs.promises.stat(briefPath);
      if (st.size > 0) return { ok: true, path: briefPath };
    } catch {
      // not written yet
    }
    await new Promise((r) => setTimeout(r, POLL_MS));
  }
  return fallbackMechanical(sessionId, 'source agent did not write the brief in time');
}
