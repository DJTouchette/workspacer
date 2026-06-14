// Must run before anything touches window.electronAPI: on web (no Electron
// preload) this installs the hub-bus-backed backend under the same global.
// On desktop it's a no-op.
import './backend/install'
import React from 'react'
import ReactDOM from 'react-dom/client'
import App from './App'
import { ConfigProvider } from './contexts/ConfigContext'
import ErrorBoundary from './components/ErrorBoundary'

ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
  <React.StrictMode>
    {/* Last-resort boundary: a crash above the app shell still shows a
        recoverable screen instead of a blank window. */}
    <ErrorBoundary label="Workspacer" variant="region">
      <ConfigProvider>
        <App />
      </ConfigProvider>
    </ErrorBoundary>
  </React.StrictMode>,
)
