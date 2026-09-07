import React from 'react';
import type { Config } from '../../hooks/useConfig';
import type { AgentProvider } from '../../types/pane';
import { useProviderDetection } from '../../hooks/useProviderDetection';
import { visibleProviderOptions, NOT_INSTALLED_SUFFIX } from '../../lib/providerAvailability';
import { CheckRow, Row, ModeButton, SearchableSelect } from './primitives';
import HarnessModelSelect from './HarnessModelSelect';

const PROVIDERS: { label: string; value: AgentProvider }[] = [
  { label: 'Anthropic / Claude', value: 'claude' },
  { label: 'Codex', value: 'codex' },
  { label: 'Copilot', value: 'copilot' },
  { label: 'OpenCode', value: 'opencode' },
  { label: 'Pi', value: 'pi' },
];
export default function StatusSummarySettings({
  config,
  save,
}: {
  config: Config;
  save: (partial: Partial<Config>) => Promise<Config>;
}) {
  const summary = config.agents?.statusSummary ?? {
    enabled: true,
    provider: 'claude',
    model: 'haiku',
  };
  const provider = summary.provider ?? 'claude';
  const { detection } = useProviderDetection();
  const providers = visibleProviderOptions(PROVIDERS, detection, [provider]);
  const patch = (value: Partial<typeof summary>) =>
    save({ agents: { ...config.agents, statusSummary: { ...summary, ...value } } });
  return (
    <div
      className="wks-status-summary"
      data-testid="status-summary-settings"
      style={{ display: 'flex', flexDirection: 'column', gap: 8, minWidth: 0 }}
    >
      <style>{`.wks-status-summary > label { flex-wrap: wrap; }`}</style>
      <CheckRow
        label="Summarize agent status on demand"
        checked={summary.enabled !== false}
        onChange={(enabled) => patch({ enabled })}
      />
      <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-secondary)' }}>
        Answer “what is this agent doing?” with a compact model summary. Uses existing CLI login;
        runs only when requested through summarize_agent_status. No automatic polling. Summaries are
        interpretations, not completion or blocker evidence. Requires a compatible desktop and
        daemon on the source hub.
      </div>
      <Row label="Status summary provider" wrap>
        <div style={{ display: 'flex', flexWrap: 'wrap', gap: 4, minWidth: 0 }}>
          {providers.map((p) => (
            <ModeButton
              key={p.value}
              label={p.missing ? `${p.label}${NOT_INSTALLED_SUFFIX}` : p.label}
              active={provider === p.value}
              onClick={() => patch({ provider: p.value, model: null })}
            />
          ))}
        </div>
      </Row>
      {provider === 'copilot' ? (
        <Row label="Status summary model" wrap>
          <SearchableSelect
            value={summary.model ?? ''}
            options={[
              { value: '', label: 'Harness default' },
              { value: 'auto', label: 'Auto' },
              ...(summary.model && summary.model !== 'auto'
                ? [{ value: summary.model, label: `${summary.model} (unsupported)` }]
                : []),
            ]}
            onChange={(model) => patch({ model: model || null })}
          />
        </Row>
      ) : (
        <HarnessModelSelect
          provider={provider}
          label="Status summary model"
          value={summary.model ?? ''}
          onChange={(model) => patch({ model: model || null })}
          defaultLabel="Harness default (cost varies)"
          wrap
          refuseForeignModel
          warningSuffix="An incompatible model returns unavailable; no provider or model fallback."
        />
      )}
      {(provider === 'codex' || provider === 'opencode') && (
        <div style={{ fontSize: '0.72rem', color: 'var(--wks-warning)' }}>
          This adapter cannot yet enforce a tool-free completion. Requests return
          no-tools-unsupported without a model call.
        </div>
      )}
    </div>
  );
}
