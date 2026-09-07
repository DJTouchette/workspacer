/** Settings-only fixture. All provider detection, model lookup and saves are local. */
import React, { useState } from 'react';
import ReactDOM from 'react-dom/client';
import '../App.css';
import { DEFAULT_CONFIG } from '../hooks/configDefaults';
import type { Config } from '../hooks/useConfig';
import StatusSummarySettings from '../components/settings/StatusSummarySettings';

(window as any).electronAPI = {
  providerCheckAll: async () =>
    ['claude', 'codex', 'copilot', 'opencode', 'pi'].map((provider) => ({
      provider,
      found: true,
      resolvedPath: `/fake/${provider}`,
      customBin: '',
    })),
  onConfigChanged: () => () => {},
  claudeListModels: async () => ({
    aliases: [
      { value: 'haiku', label: 'Haiku' },
      { value: 'sonnet', label: 'Sonnet' },
    ],
    seen: [],
    defaultModel: '',
  }),
  providerListModels: async () => [{ id: 'gpt-5-nano', label: 'GPT-5 Nano' }],
};
function Harness() {
  const [config, setConfig] = useState<Config>(DEFAULT_CONFIG);
  const save = async (partial: Partial<Config>) => {
    const next = { ...config, ...partial };
    setConfig(next);
    return next;
  };
  return (
    <main style={{ padding: 16, maxWidth: 760, margin: '0 auto' }}>
      <StatusSummarySettings config={config} save={save} />
      <output data-testid="saved-summary" style={{ display: 'block', overflowWrap: 'anywhere' }}>
        {JSON.stringify(config.agents?.statusSummary)}
      </output>
    </main>
  );
}
ReactDOM.createRoot(document.getElementById('root')!).render(<Harness />);
