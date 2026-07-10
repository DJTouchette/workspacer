/**
 * Cross-component bus for deep-linking into a Settings section.
 *
 * A caller (e.g. the welcome card's "customize your keybinds") asks for a
 * section by key; App opens/focuses the Settings pane and the pane selects the
 * section. The pending value covers the mount race — the request usually fires
 * in the same tick the pane is created, before its listener exists.
 */

export const SETTINGS_SECTION_EVENT = 'settings:open-section';

let pendingSection: string | null = null;

/** Ask the Settings pane to show a section (e.g. 'keybindings'). */
export function requestSettingsSection(key: string): void {
  pendingSection = key;
  window.dispatchEvent(new CustomEvent(SETTINGS_SECTION_EVENT, { detail: { key } }));
}

/** One-shot read of a request that fired before the pane mounted. */
export function consumePendingSettingsSection(): string | null {
  const key = pendingSection;
  pendingSection = null;
  return key;
}
