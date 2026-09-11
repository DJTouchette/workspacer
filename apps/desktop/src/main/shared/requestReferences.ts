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

function inferredReference(
  url: string,
): Extract<TaskReferenceUpsert, { kind: 'pullRequest' }> | undefined {
  const parsed = new URL(url);
  const knownProvider =
    parsed.hostname === 'github.com' ||
    parsed.hostname === 'gitlab.com' ||
    parsed.hostname === 'dev.azure.com' ||
    parsed.hostname.endsWith('.visualstudio.com') ||
    parsed.pathname.includes('/-/merge_requests/');
  if (!knownProvider) return undefined;
  const number = parsed.pathname.match(
    /(?:\/pull\/|\/merge_requests\/|\/_git\/[^/]+\/pullrequest\/)([1-9][0-9]{0,9})\/?$/i,
  )?.[1];
  return number ? { kind: 'pullRequest', number, url } : undefined;
}

// Treat Unicode letters, marks, digits, connectors, dashes and join controls as
// identifier continuations. JS \b would accept TASK-1 inside TASK-1-extra/éTASK-1.
const IDENTIFIER_PART = String.raw`[\p{L}\p{N}\p{M}\p{Pc}\p{Pd}\u200c\u200d]`;
function containsIdentifier(content: string, identifier: string): boolean {
  const escaped = identifier.trim().replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  return new RegExp(`(?<!${IDENTIFIER_PART})${escaped}(?!${IDENTIFIER_PART})`, 'u').test(content);
}

function explicitPrNumbers(content: string): Set<string> {
  // URL path/query words are not prose PR declarations. Only the known URL
  // parser above may derive identifiers from URLs.
  const prose = content.replace(/https?:\/\/[^\s<>"`]+/gi, ' ');
  const pattern = new RegExp(
    `(?<!${IDENTIFIER_PART})(?:PR|MR|pull\\s+request|merge\\s+request)\\s*(?:[#!]|(?:number|no\\.?)\\s+)?\\s*([1-9][0-9]{0,9})(?!${IDENTIFIER_PART})`,
    'giu',
  );
  return new Set([...prose.matchAll(pattern)].map((match) => match[1]));
}

/** Resolve the mapping before any task/content mutation. Empty arrays explicitly leave URLs unassigned. */
export function mapRequestReferences(
  content: string,
  intents: RequestIntent[],
): Map<string, TaskReferenceUpsert[]> {
  const urls = requestUrls(content);
  const work = intents.filter((i) => ['create', 'followUp', 'update'].includes(i.kind));
  const result = new Map<string, TaskReferenceUpsert[]>();
  if (!work.length) return result;
  const sourceNumbers = explicitPrNumbers(content);
  for (const url of urls) {
    const number = inferredReference(url)?.number;
    if (number) sourceNumbers.add(number);
  }
  const explicit = work.some((i) => i.references !== undefined);
  if (urls.length && !explicit) {
    const reference = urls.length === 1 ? inferredReference(urls[0]) : undefined;
    if (work.length !== 1 || !reference)
      throw new Error(
        'Reference mapping required: supply references on every work intent (an empty array leaves URLs unassigned). Original request content is retained.',
      );
    result.set(work[0].key, [reference]);
    return result;
  }
  for (const intent of work) {
    if (explicit && intent.references === undefined)
      throw new Error(
        'Reference mapping required on every work intent; use an empty array where none belong.',
      );
    const refs: TaskReferenceUpsert[] = [];
    let pullRequest: string | undefined;
    for (const ref of intent.references ?? []) {
      // Validate entries individually: combining first can hide invalid earlier
      // singleton values behind a later overwrite.
      const validated = applyTaskReferences(undefined, [ref], undefined);
      if (ref.url && !urls.includes(validateTaskUrl(ref.url)))
        throw new Error('Mapped reference URL must occur in the original submitted request');
      if (ref.kind === 'pullRequest') {
        const parsed = ref.url ? inferredReference(validateTaskUrl(ref.url)) : undefined;
        if (ref.url && !parsed)
          throw new Error(
            'PR URL must identify a supported original pull/merge request; use a generic reference for other URLs',
          );
        if (ref.number !== undefined) {
          if (!sourceNumbers.has(ref.number))
            throw new Error(
              'PR number must match an explicit original PR/MR identifier or original PR URL',
            );
          if (ref.url && parsed?.number !== ref.number)
            throw new Error('PR URL and number must identify the same original pull request');
        }
        const identity = JSON.stringify(validated.pullRequest);
        if (pullRequest !== undefined) {
          if (pullRequest !== identity)
            throw new Error(
              'Map at most one distinct PR per task; replacement cannot discard other mapped PRs',
            );
          continue; // Exact validated duplicates are idempotent, not replacements.
        }
        pullRequest = identity;
      }
      if (ref.kind === 'ticket' && !containsIdentifier(content, ref.id))
        throw new Error(
          'Explicit ticket ID must occur as a whole identifier in the original submitted request',
        );
      refs.push(ref);
    }
    if (refs.length) applyTaskReferences(undefined, refs, undefined);
    result.set(intent.key, refs);
  }
  return result;
}

/** Preserve human edits, including differently named references to the same URL. */
export function attachRequestReferences(
  current: TaskLinks | undefined,
  refs: TaskReferenceUpsert[],
  replace = false,
): TaskLinks | undefined {
  let links = current;
  for (const ref of refs) {
    const url = ref.url ? validateTaskUrl(ref.url) : undefined;
    const existingUrls = [
      links?.pullRequest?.url,
      ...(links?.tickets ?? []).map((r) => r.url),
      ...(links?.references ?? []).map((r) => r.url),
    ];
    if (url && existingUrls.includes(url)) continue;
    if (ref.kind === 'pullRequest' && links?.pullRequest) {
      if (!replace)
        throw new Error(
          'Task already has a different PR. Explicit replacePullRequest intent is required; original request retained.',
        );
      links = applyTaskReferences(links, [ref], [{ kind: 'pullRequest' }]);
    } else {
      if (
        ref.kind === 'reference' &&
        links?.references?.some((r) => r.label.toLowerCase() === ref.label.trim().toLowerCase())
      )
        throw new Error('Reference label already belongs to another URL; choose a distinct label');
      if (
        ref.kind === 'ticket' &&
        links?.tickets?.some(
          (r) => r.id.toLowerCase() === ref.id.trim().toLowerCase() && r.url && r.url !== url,
        )
      )
        throw new Error('Ticket already has a different URL');
      links = applyTaskReferences(links, [ref], undefined);
    }
  }
  return links;
}
