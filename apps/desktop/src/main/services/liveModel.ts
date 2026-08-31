/**
 * Pair-aware live model switching shared by desktop IPC and the hub capability.
 * The daemon owns acceptance; this layer validates with the same TS contract,
 * forwards both wire generations, and records only the accepted owner result.
 */

import { resolveSpawnModelInput, type SpawnModelInput } from '../lib/spawnModel';
import {
  claudeArgvModel,
  ModelSelectionError,
  type ModelSelection,
} from '../shared/modelContextWindows';
import { claudeSessionStore } from './claudeSessionStore';
import { claudemonSessionClient } from './claudemonSessionClient';

export interface LiveModelSwitchInput extends SpawnModelInput {
  effort?: string;
}

export interface LiveModelSwitchResult {
  ok: boolean;
  error?: string;
  model?: string;
  requestedSelection?: ModelSelection;
  queued?: boolean;
  disposition?: 'queued' | 'accepted';
}

function normalizedRequest(
  sessionId: string,
  input: LiveModelSwitchInput,
): { selection?: ModelSelection; legacyModel?: string } {
  const provider = claudeSessionStore.getSnapshot(sessionId)?.provider ?? 'claude';
  const selection = resolveSpawnModelInput(provider, input);
  if (
    (input.model != null || input.modelIdentity != null || input.contextWindow != null) &&
    !selection
  ) {
    throw new ModelSelectionError('empty-model', 'model selection must name a model');
  }
  return {
    selection,
    legacyModel: selection
      ? provider.toLowerCase() === 'claude'
        ? claudeArgvModel(selection)
        : input.model?.trim() || selection.model
      : undefined,
  };
}

/** Apply an owner response to the local mirror. The local normalized request is
 * used only when talking to an older owner that cannot return the additive pair. */
export function acceptLiveModelResult(
  sessionId: string,
  input: LiveModelSwitchInput,
  result: LiveModelSwitchResult,
): LiveModelSwitchResult {
  if (!result.ok) return result;
  const snapshot = claudeSessionStore.getSnapshot(sessionId);
  if (
    (snapshot?.provider ?? 'claude').toLowerCase() === 'claude' &&
    snapshot?.transport !== 'stream' &&
    result.queued === undefined &&
    result.disposition === undefined
  ) {
    return {
      ok: false,
      error:
        'upgrade-required: the session owner does not support durable Claude PTY model switching',
    };
  }
  const request = normalizedRequest(sessionId, input);
  const accepted = result.requestedSelection ?? request.selection;
  if (accepted) {
    claudeSessionStore.noteRequestedModelSelection(
      sessionId,
      accepted,
      result.model ?? request.legacyModel,
    );
  }
  return {
    ...result,
    ...(accepted && { requestedSelection: accepted }),
    ...((result.model ?? request.legacyModel)
      ? { model: result.model ?? request.legacyModel }
      : {}),
  };
}

export async function applyLiveModel(
  sessionId: string,
  input: LiveModelSwitchInput,
): Promise<LiveModelSwitchResult> {
  if (!sessionId) return { ok: false, error: 'requires a session' };
  try {
    const request = normalizedRequest(sessionId, input);
    const effort = input.effort?.trim() || undefined;
    if (!request.selection && !effort) {
      return { ok: false, error: 'requires a model and/or effort' };
    }
    const snapshot = claudeSessionStore.getSnapshot(sessionId);
    if (
      request.selection &&
      effort !== undefined &&
      (snapshot?.provider ?? 'claude').toLowerCase() === 'claude' &&
      snapshot?.transport !== 'stream'
    ) {
      return {
        ok: false,
        error: 'claude-pty-effort-unsupported: the PTY model command cannot deliver effort',
      };
    }
    const result = await claudemonSessionClient.setModel(
      sessionId,
      request.legacyModel,
      effort,
      request.selection?.model,
      request.selection?.contextWindow,
    );
    return acceptLiveModelResult(sessionId, input, result);
  } catch (err) {
    return {
      ok: false,
      error:
        err instanceof ModelSelectionError
          ? err.code
          : err instanceof Error
            ? err.message
            : String(err),
    };
  }
}
