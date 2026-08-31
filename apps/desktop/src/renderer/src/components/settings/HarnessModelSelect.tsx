/**
 * A model picker that follows the HARNESS it is picking for.
 *
 * Every model setting in this app used to source its dropdown from
 * `claudeListModels()` regardless of which harness the thing being configured
 * ran on, so choosing Codex left you picking from `fable`/`opus`/`sonnet` and
 * saving one wrote an id the codex CLI rejects — the field looked configured and
 * the spawn 400'd. Fixing that once per picker is how the next picker gets it
 * wrong again, so the whole behaviour lives here:
 *
 *  - Options come from `loadModelOptions(provider, capsFor(provider).modelSource)`,
 *    the same source the spawn dialog and the composer pill read: Claude's
 *    curated aliases + ids seen across sessions, or the daemon's live
 *    per-provider catalog (which boots that CLI to ask it). Never throws — an
 *    uninstalled or unauthed harness resolves to an empty list, which renders as
 *    "harness default only" rather than an error.
 *  - A value the harness's catalog doesn't offer is kept VISIBLE and flagged,
 *    never dropped: silently blanking the field would misreport what the config
 *    still holds. The warning distinguishes "another harness's id" (which the
 *    spawn paths now refuse outright, per main/shared/modelVocabulary) from
 *    merely "not in the catalog" (a retired model, a private deployment, a
 *    catalog that failed to load) — the first is a bug in the setting, the
 *    second may be perfectly deliberate.
 *
 * `provider` is which harness this row is for, NOT necessarily the one config
 * calls default — the auto-title row renders one of these per harness at once,
 * because every agent is titled by its own.
 */
import React, { useEffect, useState } from 'react';
import type { AgentProvider } from '../../types/pane';
import { capsFor } from '../../lib/providerCaps';
import { loadModelOptions, modelOptionCommand } from '../../lib/modelOptions';
import { isForeignModel } from '../../../../main/shared/modelVocabulary';
import { Row, SearchableSelect, SelectOption } from './primitives';

/** The pickable models for one harness. Exported: SupervisorSection reuses it
 *  for the supervisor's own coordinator row. */
export function useModelOptions(provider: AgentProvider): {
  options: SelectOption[];
  loaded: boolean;
} {
  const [options, setOptions] = useState<SelectOption[]>([]);
  const [loaded, setLoaded] = useState(false);
  useEffect(() => {
    let cancelled = false;
    setLoaded(false);
    setOptions([]);
    void loadModelOptions(provider, capsFor(provider).modelSource).then((list) => {
      if (cancelled) return;
      setOptions(
        list.map((m) => ({
          value: capsFor(provider).modelSource === 'claude' ? modelOptionCommand(m) : m.id,
          label: m.label,
        })),
      );
      setLoaded(true);
    });
    return () => {
      cancelled = true;
    };
  }, [provider]);
  return { options, loaded };
}

/**
 * Build the option list and the stale-value verdict for one harness.
 *
 * Pure and exported because this is the part worth pinning: that the current
 * value survives a catalog that doesn't know it, and that a foreign id is called
 * a foreign id rather than a missing one.
 */
export function harnessModelOptions(
  provider: AgentProvider,
  value: string,
  catalog: SelectOption[],
  loaded: boolean,
  defaultLabel: string,
): { options: SelectOption[]; unknown: boolean; foreign: boolean } {
  const known = catalog.some((o) => o.value === value);
  return {
    options: [
      { value: '', label: defaultLabel },
      ...catalog,
      ...(value && !known ? [{ value, label: `${value} (not in ${provider}’s catalog)` }] : []),
    ],
    unknown: !!value && loaded && !known,
    foreign: !!value && isForeignModel(provider, value),
  };
}

interface HarnessModelSelectProps {
  /** The harness this row picks a model for. */
  provider: AgentProvider;
  label: string;
  value: string;
  onChange: (value: string) => void;
  /** What an empty value means, in words. */
  defaultLabel?: string;
  /** Explanatory copy under the row. */
  hint?: React.ReactNode;
  /** Sentence appended to the stale-value warning, naming the consequence. */
  warningSuffix?: string;
}

const HarnessModelSelect: React.FC<HarnessModelSelectProps> = ({
  provider,
  label,
  value,
  onChange,
  defaultLabel = 'Harness default',
  hint,
  warningSuffix,
}) => {
  const { options: catalog, loaded } = useModelOptions(provider);
  const { options, unknown, foreign } = harnessModelOptions(
    provider,
    value,
    catalog,
    loaded,
    defaultLabel,
  );

  return (
    <>
      <Row label={label}>
        <SearchableSelect
          value={value}
          options={options}
          onChange={onChange}
          placeholder={defaultLabel}
        />
      </Row>
      {hint && <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-disabled)' }}>{hint}</div>}
      {(unknown || foreign) && (
        <div style={{ fontSize: '0.72rem', color: 'var(--wks-warning)' }}>
          <strong>{value}</strong>{' '}
          {foreign
            ? `belongs to a different harness, so a ${provider} spawn drops it and uses ${provider}’s own default.`
            : `is not in ${provider}’s model list, so a ${provider} spawn may be refused it.`}{' '}
          {warningSuffix ?? `Pick one above, or leave it on the ${provider} default.`}
        </div>
      )}
    </>
  );
};

export default HarnessModelSelect;
