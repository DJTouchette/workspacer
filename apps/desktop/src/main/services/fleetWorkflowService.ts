// Desktop inputs for the shared workflow implementation.
import * as runtime from './fleetWorkflowRuntime';
import { createFleetWorkflowService } from './fleetWorkflowServiceCore';
import { claudeSessionStore } from './claudeSessionStore';
import { libraryService } from './libraryService';
import { getConfigDir } from './configService';
import fs from 'fs';
import path from 'path';
import { dispatchTemplateParams } from '../lib/dispatchTemplate';
import type { WorkflowTemplate } from '../shared/fleetWorkflow';
const templates = (): WorkflowTemplate[] =>
  libraryService
    .list(undefined, (full) => {
      try {
        const root = fs.realpathSync(path.join(getConfigDir(), 'library'));
        const resolved = fs.realpathSync(full);
        const rel = path.relative(root, resolved);
        return rel && !rel.startsWith('..' + path.sep) && rel !== '..' && !path.isAbsolute(rel)
          ? resolved
          : null;
      } catch {
        return null;
      }
    })
    .filter((t) => t.kind === 'dispatch' && t.scope === 'global')
    .map((t) => ({
      id: t.id,
      body: t.body,
      resultSchema: t.resultSchema ?? {},
      params: dispatchTemplateParams(t.body),
    }));

export const { fleetWorkflowStore, fleetWorkflowRequest } = createFleetWorkflowService(runtime, (id) => claudeSessionStore.getSnapshot(id) ?? undefined, templates);
