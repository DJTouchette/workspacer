/**
 * Pair-aware live model switching shared by desktop IPC and the hub capability.
 * The daemon owns acceptance; this layer validates with the same TS contract,
 * forwards both wire generations, and records only the accepted owner result.
 */

import { resolveSpawnModelInput, type SpawnModelInput } from '../lib/spawnModel';
import { claudeArgvModel, type ModelSelection } from '../shared/modelContextWindows';
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
}

function normalizedRequest(
  sessionId: string,
  input: LiveModelSwitchInput,
): { selection?: ModelSelection; legacyModel?: string } {
  const provider = claudeSessionStore.getSnapshot(sessionId)?.provider ?? 'claude';
  const selection = resolveSpawnModelInput(provider, input);
  if (
    (input.model?.trim() || input.modelIdentity?.trim() || input.contextWindow != null) &&
    !selection
  ) {
    throw new Error(`model is not valid for provider ${provider}`);
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
    if (!request.selection && !input.effort?.trim()) {
      return { ok: false, error: 'requires a model and/or effort' };
    }
    const result = await claudemonSessionClient.setModel(
      sessionId,
      request.legacyModel,
      input.effort,
      request.selection?.model,
      request.selection?.contextWindow,
    );
    return acceptLiveModelResult(sessionId, input, result);
  } catch (err) {
    return { ok: false, error: err instanceof Error ? err.message : String(err) };
  }
}
