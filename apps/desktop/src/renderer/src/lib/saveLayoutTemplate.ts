import type { LayoutAgent } from '../types/layout';

/**
 * Persist a layout template.
 *
 * Deliberately a named function rather than an inline `.catch(console.error)` in
 * App: the rejection IS the product. Swallowing it returned `undefined`, which
 * made LayoutsDialog take its non-Promise branch — clear the name field, run the
 * success animation, reload a list that simply lacks the entry — so a failed
 * save was pixel-identical to a successful one and console.error, which has no
 * consumer in a packaged app, was the only trace.
 */
export function saveLayoutTemplate(name: string, agents: LayoutAgent[]): Promise<void> {
  return window.electronAPI.layoutsSave({ name, agents }).then(() => undefined);
}
