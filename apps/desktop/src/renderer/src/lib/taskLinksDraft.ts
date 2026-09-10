import { validateTaskLinks, type TaskLinks } from '../../../main/shared/dispatchHistory';

/** Origins belong only to the open editor. Renaming a row must not turn it into
 * an unrelated delete/add, or a concurrent URL edit could be lost. */
export interface TaskLinksDraft {
  base: TaskLinks;
  links: TaskLinks;
  ticketOrigins: Array<string | undefined>;
  referenceOrigins: Array<string | undefined>;
}
const identity = (name: string): string => name.trim().toLowerCase();

export function createTaskLinksDraft(links: TaskLinks = {}): TaskLinksDraft {
  return {
    base: links,
    links,
    ticketOrigins: (links.tickets ?? []).map((r) => identity(r.id)),
    referenceOrigins: (links.references ?? []).map((r) => identity(r.label)),
  };
}

export function normalizedDraftLinks(links: TaskLinks): TaskLinks {
  const normalized = { ...links };
  if (!normalized.pullRequest?.number && !normalized.pullRequest?.url)
    delete normalized.pullRequest;
  return validateTaskLinks(normalized);
}

/** All-or-nothing three-way merge. No revision advances here and no write is
 * authorized: the resulting draft still goes through the host's ordinary CAS. */
export function rebaseTaskLinksDraft(
  draft: TaskLinksDraft,
  current: TaskLinks = {},
): TaskLinksDraft {
  const desired = normalizedDraftLinks(draft.links);
  const collision = (field: string): never => {
    throw new Error(
      `Both edits changed ${field}. Your draft is preserved. Resolve that field in the draft and try again, or Reload references to use the current links.`,
    );
  };
  const mergeValue = (
    base: string | undefined,
    value: string | undefined,
    now: string | undefined,
    field: string,
  ) => {
    if (value === base) return now;
    if (now === base || now === value) return value;
    return collision(`${field} (current value: ${now ?? 'empty'})`);
  };
  const mergeEntry = <T extends object>(
    base: T | undefined,
    value: T | undefined,
    now: T | undefined,
    fields: Array<keyof T>,
    label: string,
  ): T | undefined => {
    const equal = (a: T | undefined, b: T | undefined) =>
      fields.every((key) => a?.[key] === b?.[key]);
    if (!value) {
      if (!base) return now;
      if (!now || equal(base, now)) return undefined;
      return collision(`${label} (removed in your draft)`);
    }
    if (base && !now) {
      if (equal(base, value)) return undefined;
      return collision(`${label} (removed from the current task)`);
    }
    return Object.fromEntries(
      fields.map((key) => [
        key,
        mergeValue(
          base?.[key] as string | undefined,
          value[key] as string | undefined,
          now?.[key] as string | undefined,
          `${label} ${String(key)}`,
        ),
      ]),
    ) as T;
  };
  const mergeRows = <T extends object>(
    base: T[],
    values: T[],
    currentRows: T[],
    origins: Array<string | undefined>,
    name: (row: T) => string,
    fields: Array<keyof T>,
    label: string,
  ) => {
    const rows: T[] = [];
    const nextOrigins: Array<string | undefined> = [];
    const handled = new Set<string>();
    const key = (row: T) => identity(name(row));
    const append = (row: T | undefined, origin: string | undefined) => {
      if (row) {
        rows.push(row);
        nextOrigins.push(origin);
      }
    };
    for (const before of base) {
      const origin = key(before);
      const index = origins.indexOf(origin);
      const now = currentRows.find((r) => key(r) === origin);
      handled.add(origin);
      append(
        mergeEntry(
          before,
          index < 0 ? undefined : values[index],
          now,
          fields,
          `${label} “${name(before)}”`,
        ),
        now ? origin : undefined,
      );
    }
    values.forEach((value, index) => {
      if (origins[index] !== undefined) return;
      const origin = key(value);
      const now = currentRows.find((r) => key(r) === origin);
      // A new row reusing an explicitly removed identity is still a collision
      // if the current entry changed; it must not bypass the removal check.
      if (handled.has(origin)) collision(`${label} “${name(value)}” identity`);
      handled.add(origin);
      append(
        mergeEntry(undefined, value, now, fields, `${label} “${name(value)}”`),
        now ? origin : undefined,
      );
    });
    for (const now of currentRows) {
      if (!handled.has(key(now))) append(now, key(now));
    }
    return { rows, origins: nextOrigins };
  };
  const tickets = mergeRows(
    draft.base.tickets ?? [],
    desired.tickets ?? [],
    current.tickets ?? [],
    draft.ticketOrigins,
    (r) => r.id,
    ['id', 'url'],
    'ticket',
  );
  const references = mergeRows(
    draft.base.references ?? [],
    desired.references ?? [],
    current.references ?? [],
    draft.referenceOrigins,
    (r) => r.label,
    ['label', 'url'],
    'reference',
  );
  const pullRequest = mergeEntry(
    draft.base.pullRequest,
    desired.pullRequest,
    current.pullRequest,
    ['number', 'url'],
    'PR',
  );
  const links = validateTaskLinks({
    ...(pullRequest ? { pullRequest } : {}),
    tickets: tickets.rows,
    references: references.rows,
  });
  return {
    base: current,
    links,
    ticketOrigins: tickets.origins,
    referenceOrigins: references.origins,
  };
}
