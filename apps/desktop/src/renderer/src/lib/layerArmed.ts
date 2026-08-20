/**
 * Window-level "chord leader is armed" flag.
 *
 * Owned by useKeyboardNav (set whenever the chord path is non-null) and read
 * by surfaces that must change key routing while a chord is mid-flight — the
 * one today is BrowserPane, which forwards ALL guest keys to the host while
 * armed so a chord step typed with focus inside a webview reaches the window
 * dispatcher instead of the page. A module-level flag rather than React state
 * because the readers are inside native event handlers (before-input-event)
 * where a render-cycle subscription would race the very keystroke it gates.
 */
let armed = false;

export function setLayerArmed(value: boolean): void {
  armed = value;
}

export function isLayerArmed(): boolean {
  return armed;
}
