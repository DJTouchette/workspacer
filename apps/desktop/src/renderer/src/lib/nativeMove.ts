/**
 * The Electron app's sunset notice. v0.170.0 is the last Electron release;
 * Workspacer ships only as the native (Rust/GPUI) app after it. Existing users
 * reach this build through electron-updater, so it has to tell them where the
 * product went.
 *
 * The native app keeps its own store, but a fresh native install imports this
 * app's config, hub state and claudemon sessions on first launch
 * (apps/native/src/host/electron_import.rs); `wks-native --local
 * --import-electron` re-runs it over an existing native install.
 */
export const NATIVE_DOWNLOAD_URL = 'https://github.com/DJTouchette/workspacer/releases/latest';

export const NATIVE_MOVE_TITLE = 'Workspacer has moved to a native app';
export const NATIVE_MOVE_DETAIL =
  'This is the final Electron release and it will not receive further updates. ' +
  'On first launch the native app imports your settings and sessions from this app.';
export const NATIVE_DOWNLOAD_LABEL = 'Download Workspacer Native';

/** Open the release page in the default browser, never in the webview. */
export function openNativeDownload(): void {
  void window.electronAPI.openExternalUrl?.(NATIVE_DOWNLOAD_URL);
}
