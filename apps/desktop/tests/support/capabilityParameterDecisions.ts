/** Shared source-authority decisions consumed by the TS guards and Rust AST tool.
 * This test policy describes values; it does not create runtime grants. */
import policy from '../fixtures/capability-parameter-policy.json';
export const pathParameters: Record<string, string> = policy.pathParameters;
export const methodDecisions: Record<string, string> = policy.methodDecisions;
export const inertMethods: Record<string, string> = policy.inertMethods;
export const parameterDecisions: Record<
  string,
  Record<string, { kind: string; reason: string }>
> = policy.parameterDecisions;
export const dangerousNames: Record<string, string> = policy.dangerousNames;
export const pathNamespaces: string[] = policy.pathNamespaces;
