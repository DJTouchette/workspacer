/**
 * Re-export the auto-generated BrowserService bindings.
 * Wails v3 generates these with Call.ByID for optimal performance.
 *
 * NOTE: The import below will fail until the Go backend is built
 * and Wails generates the bindings. This is expected.
 */
// @ts-ignore — bindings not yet generated
export {
  OpenBrowser,
  CloseBrowser,
  GetBrowserURL,
} from '../../bindings/workspacer/browserservice';
