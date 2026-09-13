import { parentPort, workerData, isMainThread } from 'node:worker_threads';
import { DatabaseSync } from 'node:sqlite';
import { IntentWorkspaceStore } from './intentWorkspaceStore';
import {
  isIntentFileAction,
  type IntentFileWorkerRequest,
  type IntentFileWorkerResponse,
} from './intentFileWorkerProtocol';
import { setIntentWindowsFileDeadline } from './intentWindowsFiles';

if (isMainThread || !parentPort || typeof workerData?.filename !== 'string')
  throw new Error('Intent file worker requires its host-owned database');
const db = new DatabaseSync(workerData.filename);
const store = new IntentWorkspaceStore(db);
let active = false;
parentPort.on('message', async (message: IntentFileWorkerRequest) => {
  const token = message?.token;
  let response: IntentFileWorkerResponse;
  if (active) {
    parentPort!.postMessage({ token, error: 'Intent file worker is already busy' });
    return;
  }
  active = true;
  try {
    if (
      !message ||
      !isIntentFileAction(message.request) ||
      !Number.isFinite(message.expiresAt) ||
      message.expiresAt <= Date.now()
    )
      throw new Error('Invalid or expired intent file request');
    setIntentWindowsFileDeadline(message.expiresAt);
    const result =
      message.request.action === 'captureEvidence'
        ? await store.evidence.capture(message.request, message.sessions)
        : store.request(message.request);
    response = { token, result };
  } catch (error) {
    response = { token, error: error instanceof Error ? error.message : String(error) };
  } finally {
    active = false;
    setIntentWindowsFileDeadline(undefined);
  }
  parentPort!.postMessage(response);
});
