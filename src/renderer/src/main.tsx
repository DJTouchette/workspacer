// Must run before anything touches window.electronAPI: on web (no Electron
// preload) this installs the hub-bus-backed backend under the same global.
// On desktop it's a no-op.
import './backend/install'
import React from 'react'
import ReactDOM from 'react-dom/client'
import App from './App'
import { ConfigProvider } from './contexts/ConfigContext'

ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
  <React.StrictMode>
    <ConfigProvider>
      <App />
    </ConfigProvider>
  </React.StrictMode>,
)
