/** Read-only host metadata. Absence means an older peer, never readiness. */
export interface ExecutionEngineMetadata {
  id: string;
  api_version: number;
  implementation_version: string;
  generation: number;
  readiness: string;
}
