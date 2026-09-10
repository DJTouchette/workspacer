import {
  applyTaskReferences,
  validateTaskUrl,
  type TaskLinks,
  type TaskReferenceUpsert,
} from './dispatchHistory';
import type { RequestIntent } from './managerRequests';

/** Only submitted text is inspected. No clipboard, URL fetches, or ticket inference. */
export function requestUrls(content: string): string[] {
  const urls = new Set<string>();
  for (const match of content.matchAll(/https?:\/\/[^\s<>"`]+/gi)) {
    const candidate = match[0].replace(/[.,;:!?\])}]+$/, '');
    try {
      urls.add(validateTaskUrl(candidate));
    } catch {
      // Unsafe links remain in original content; they never become openable references.
    }
  }
  return [...urls];
}

function inferredReference(url: string): TaskReferenceUpsert | undefined {
  const parsed = new URL(url);
  const knownProvider = parsed.hostname === 'github.com' || parsed.hostname === 'gitlab.com' || parsed.hostname === 'dev.azure.com' || parsed.hostname.endsWith('.visualstudio.com') || parsed.pathname.includes('/-/merge_requests/');
  if (!knownProvider) return undefined;
  const number = parsed.pathname.match(
    /(?:\/pull\/|\/merge_requests\/|\/_git\/[^/]+\/pullrequest\/)([1-9][0-9]{0,9})\/?$/i,
  )?.[1];
  return number ? { kind: 'pullRequest', number, url } : undefined;
}

/** Resolve the mapping before any task/content mutation. Empty arrays explicitly leave URLs unassigned. */
export function mapRequestReferences(content: string, intents: RequestIntent[]): Map<string, TaskReferenceUpsert[]> {
  const urls = requestUrls(content);
  const work = intents.filter((i) => i.kind !== 'none' && i.kind !== 'question');
  const result = new Map<string, TaskReferenceUpsert[]>();
  if (!work.length) return result;
  const explicit = work.some((i) => i.references !== undefined);
  if (urls.length && !explicit) {
    const reference = urls.length === 1 ? inferredReference(urls[0]) : undefined;
    if (work.length !== 1 || !reference)
      throw new Error('Reference mapping required: supply references on every work intent (an empty array leaves URLs unassigned). Original request content is retained.');
    result.set(work[0].key, [reference]);
    return result;
  }
  for (const intent of work) {
    if (explicit && intent.references === undefined)
      throw new Error('Reference mapping required on every work intent; use an empty array where none belong.');
    const refs = intent.references ?? [];
    if (refs.length) applyTaskReferences(undefined, refs, undefined);
    for (const ref of refs) {
      if (ref.url && !urls.includes(validateTaskUrl(ref.url)))
        throw new Error('Mapped reference URL must occur in the original submitted request');
      if (ref.kind === 'ticket' && !content.includes(ref.id))
        throw new Error('Explicit ticket ID must occur in the original submitted request');
    }
    result.set(intent.key, refs);
  }
  return result;
}

/** Preserve human edits, including differently named references to the same URL. */
export function attachRequestReferences(current: TaskLinks | undefined, refs: TaskReferenceUpsert[], replace = false): TaskLinks | undefined {
  let links = current;
  for (const ref of refs) {
    const url = ref.url ? validateTaskUrl(ref.url) : undefined;
    const existingUrls = [links?.pullRequest?.url, ...(links?.tickets ?? []).map((r) => r.url), ...(links?.references ?? []).map((r) => r.url)];
    if (url && existingUrls.includes(url)) continue;
    if (ref.kind === 'pullRequest' && links?.pullRequest) {
      if (!replace) throw new Error('Task already has a different PR. Explicit replacePullRequest intent is required; original request retained.');
      links = applyTaskReferences(links, [ref], [{ kind: 'pullRequest' }]);
    } else {
      if (ref.kind === 'reference' && links?.references?.some((r) => r.label.toLowerCase() === ref.label.trim().toLowerCase()))
        throw new Error('Reference label already belongs to another URL; choose a distinct label');
      if (ref.kind === 'ticket' && links?.tickets?.some((r) => r.id.toLowerCase() === ref.id.trim().toLowerCase() && r.url && r.url !== url))
        throw new Error('Ticket already has a different URL');
      links = applyTaskReferences(links, [ref], undefined);
    }
  }
  return links;
}
