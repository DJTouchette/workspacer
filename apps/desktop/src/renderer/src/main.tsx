// The backend must be installed under window.electronAPI before any app module
// is *evaluated* — modules in App's import graph may read window.electronAPI at
// load time, and on the web build it doesn't exist until installBackend() runs.
// So we install first, then dynamically import the app graph (which defers its
// evaluation until after the swap), then mount. See backend/install.
import { installBackend } from './backend/install';
import { startLongTaskMonitor } from './lib/longTaskMonitor';
import React from 'react';
import ReactDOM from 'react-dom/client';

async function mount(): Promise<void> {
  const [{ default: App }, { ConfigProvider }, { PluginsProvider }, { default: ErrorBoundary }] =
    await Promise.all([
      import('./App'),
      import('./contexts/ConfigContext'),
      import('./contexts/PluginsContext'),
      import('./components/ErrorBoundary'),
    ]);
  ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
    <React.StrictMode>
      {/* Last-resort boundary: a crash above the app shell still shows a
          recoverable screen instead of a blank window. */}
      <ErrorBoundary label="Workspacer" variant="region">
        <ConfigProvider>
          {/* One shared plugin subscription: App reads panes/hotkeys, the
              inspector rail's widget board reads widgets. */}
          <PluginsProvider>
            <App />
          </PluginsProvider>
        </ConfigProvider>
      </ErrorBoundary>
    </React.StrictMode>,
  );
}

// Started before mount so the very first render is covered too.
startLongTaskMonitor();

// Install the backend, then mount. `finally` so a failed install still renders
// (installBackend already falls back to IPC internally on any error).
installBackend().finally(() => {
  void mount();
});
