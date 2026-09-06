/** Only host-captured evidence IDs cross the transcript. Paths/revisions are never requests. */
export interface FleetReviewRequest {
  ownerSessionId: string;
  workerSessionId: string;
  evidenceId: string;
  file?: string;
}
export interface FleetReviewFile {
  path: string;
  oldPath?: string;
  status: string;
  diff?: string;
}
export interface FleetReviewEvidence {
  /** Host allocation generation; older records may lack it. */
  allocationId?: string;
  id: string;
  ownerSessionId: string;
  workerSessionId: string;
  projectRoot: string;
  allocatedCwd: string;
  branch: string;
  baseCommit: string;
  headCommit?: string;
  capturedAt: string;
  lifecycle: 'turn-ended' | 'session-ended' | 'before-worktree-removal';
  availability: 'captured' | 'dirty' | 'unavailable' | 'oversized';
  reason?: string;
  files: FleetReviewFile[];
}
export type FleetReviewResponse =
  { ok: true; evidence: FleetReviewEvidence } | { ok: false; error: string };
