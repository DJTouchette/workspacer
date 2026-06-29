import React, { useEffect, useState } from 'react';
import { Config } from '../../hooks/useConfig';
import { Section, Row, SearchableSelect, SelectOption, ModeButton, inputStyle } from './primitives';

const SUP_PROVIDERS: { value: 'claude' | 'codex' | 'opencode'; label: string }[] = [
  { value: 'claude', label: 'Claude' },
  { value: 'codex', label: 'Codex' },
  { value: 'opencode', label: 'OpenCode' },
];

interface SupervisorSectionProps {
  config: Config;
  save: (partial: Partial<Config>) => Promise<Config>;
}

/** Build the model dropdown options: the app default, the family aliases, then
 *  any concrete ids seen across sessions. Mirrors the spawn dialog's source. */
function useModelOptions(): SelectOption[] {
  const [opts, setOpts] = useState<SelectOption[]>([{ value: '', label: 'App default' }]);
  useEffect(() => {
    window.electronAPI.claudeListModels?.()
      .then((res) => {
        if (!res) return;
        const aliases = (res.aliases ?? []).map((a) => ({ value: a.value, label: a.label }));
        const seen = (res.seen ?? []).map((m) => ({ value: m, label: m }));
        setOpts([{ value: '', label: 'App default' }, ...aliases, ...seen]);
      })
      .catch(() => {});
  }, []);
  return opts;
}

const SupervisorSection: React.FC<SupervisorSectionProps> = ({ config, save }) => {
  const sup = config.supervisor ?? {};
  const model = sup.model ?? '';
  const summarizerModel = sup.summarizerModel ?? 'sonnet';
  const pollSeconds = sup.pollSeconds ?? 45;
  const modelOptions = useModelOptions();
  // The summarizer should be a cheap model — surface sonnet/haiku first, but
  // still allow any model the picker knows about.
  const summarizerOptions: SelectOption[] = modelOptions.filter((o) => o.value !== '');

  const patch = (p: Partial<NonNullable<Config['supervisor']>>) =>
    save({ supervisor: { ...sup, ...p } });

  return (
    <Section title="Supervisor">
      <div style={{ fontSize: '0.55rem', color: 'var(--wks-text-disabled)', marginBottom: 8 }}>
        Optional. The fleet supervisor is an agent you start from “Ask the Fleet” that watches your
        other agents, summarizes what they’re doing (using cheap summarizer workers), and notifies you
        when a decision is needed. Nothing here runs unless you spawn one.
      </div>

      <Row label="Supervisor agent">
        <div style={{ display: 'flex', gap: 4 }}>
          {SUP_PROVIDERS.map((p) => (
            <ModeButton
              key={p.value}
              label={p.label}
              active={(sup.provider ?? 'claude') === p.value}
              onClick={() => patch({ provider: p.value })}
            />
          ))}
        </div>
      </Row>
      <div style={{ fontSize: '0.55rem', color: 'var(--wks-text-disabled)' }}>
        Which CLI the supervisor runs on (also pickable when you launch one from “Ask the Fleet”).
        Codex and OpenCode supervisors are wired to the workspacer MCP facade — the supervisor’s
        tools to observe and coordinate the fleet — via their own MCP config. Experimental: it needs
        a CLI build with remote-MCP support; Claude remains the most battle-tested.
      </div>

      <Row label="Supervisor model">
        <SearchableSelect
          value={model}
          options={modelOptions}
          onChange={(v) => patch({ model: v })}
          placeholder="App default"
        />
      </Row>
      <div style={{ fontSize: '0.55rem', color: 'var(--wks-text-disabled)' }}>
        The coordinator model. Keep this strong — it reasons over the fleet and composes notifications.
      </div>

      <Row label="Summarizer model">
        <SearchableSelect
          value={summarizerModel}
          options={summarizerOptions}
          onChange={(v) => patch({ summarizerModel: v })}
          placeholder="sonnet"
        />
      </Row>
      <div style={{ fontSize: '0.55rem', color: 'var(--wks-text-disabled)' }}>
        The cheap model the supervisor spawns to read transcripts and write digests. Sonnet by default;
        Haiku is cheaper.
      </div>

      <Row label="Poll interval (seconds)">
        <input
          type="number"
          min={5}
          max={3600}
          value={pollSeconds}
          onChange={(e) => {
            const n = Math.round(Number(e.target.value));
            if (Number.isFinite(n) && n > 0) patch({ pollSeconds: n });
          }}
          style={{ ...inputStyle, width: 80 }}
        />
      </Row>
      <div style={{ fontSize: '0.55rem', color: 'var(--wks-text-disabled)' }}>
        How often the supervisor re-sweeps the fleet for status and pending decisions.
      </div>
    </Section>
  );
};

export default SupervisorSection;
