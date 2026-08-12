/**
 * Background-snapshot compaction.
 *
 * The implementation moved to `main/shared` so the main process can compact
 * BEFORE serializing onto the bus — the renderer used to receive every
 * session's full transcript and then throw ~99% of it away. This re-export
 * keeps the renderer's import path (and its existing tests) unchanged.
 */

export {
  compactClaudeSnapshotForBackground,
  type CompactableSnapshot,
} from '../../../main/shared/compactClaudeSnapshot';
