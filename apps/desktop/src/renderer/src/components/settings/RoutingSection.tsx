import { openPolicySettings } from '../../lib/settingsBus';
import React, { useEffect, useState } from 'react';
import { Section, SmallButton as SettingsButton, inputStyle } from './primitives';
import { Surface } from '../Surface';
import type {
  RoutingAssignment,
  RoutingPatch,
  RoutingPolicy,
  RoutingPreferencesView,
  RoutingPreview,
  RoutingValidation,
} from '../../../../main/shared/routingPreferences';

function SmallButton({
  children,
  ...props
}: {
  children: React.ReactNode;
  onClick: () => void;
  disabled?: boolean;
}) {
  return <SettingsButton {...props} label={children} />;
}

const grid: React.CSSProperties = {
  display: 'grid',
  gridTemplateColumns: 'repeat(auto-fit, minmax(min(100%, 180px), 1fr))',
  gap: 12,
};
const muted: React.CSSProperties = {
  color: 'var(--wks-text-muted)',
  fontSize: '0.72rem',
  overflowWrap: 'anywhere',
};
const selectStyle: React.CSSProperties = {
  ...inputStyle,
  width: '100%',
  minWidth: 0,
  cursor: 'pointer',
};
const roleHints: Record<string, string> = {
  scout: 'Explore code and clarify the task',
  mechanical: 'Repetitive changes with known steps',
  implementer: 'Build features and solve engineering problems',
  reviewer: 'Independent review',
  deep_reviewer: 'Architecture and complex logic review',
  fixer: 'Routine fixes',
  complex_fixer: 'Difficult fixes',
  validator: 'Check behavior and test results',
  diagnostician: 'Investigate causes',
  judge: 'Adjudicate difficult disagreements',
};
const modes = ['auto', 'normal', 'conserve', 'spend_down'];
const label = (s: string) => s.replace(/([a-z])([A-Z])/g, '$1 $2').replace(/_/g, ' ');
const clone = <T,>(v: T): T => JSON.parse(JSON.stringify(v));

/** Sparse difference: arrays replace as one value; untouched policy never rides a save. */
export function routingDifference(base: unknown, draft: unknown): RoutingPatch {
  const out: Record<string, unknown> = {};
  for (const [k, value] of Object.entries(draft as Record<string, unknown>)) {
    const prior = (base as Record<string, unknown>)[k];
    if (JSON.stringify(value) === JSON.stringify(prior)) continue;
    out[k] =
      value && prior && typeof value === 'object' && !Array.isArray(value)
        ? routingDifference(prior, value)
        : value;
  }
  return out as RoutingPatch;
}
function Pick({
  name,
  value,
  choices,
  onChange,
  disabled = false,
}: {
  name: string;
  value: string;
  choices: Array<string | { value: string; label: string }>;
  onChange: (value: string) => void;
  disabled?: boolean;
}) {
  const options = choices.map((c) => (typeof c === 'string' ? { value: c, label: label(c) } : c));
  if (!options.some((o) => o.value === value))
    options.unshift({ value, label: value || 'Provider default' });
  return (
    <label style={{ display: 'grid', gap: 4, minWidth: 0, fontSize: '0.72rem' }}>
      {name}
      <select
        aria-label={name}
        style={selectStyle}
        value={value}
        disabled={disabled}
        onChange={(e) => onChange(e.target.value)}
      >
        {options.map((o) => (
          <option key={o.value} value={o.value}>
            {o.label}
          </option>
        ))}
      </select>
    </label>
  );
}
function Source({ view, path }: { view: RoutingPreferencesView; path: string }) {
  const sources = Object.entries(view.sourceByPath)
    .filter(([k]) => k === path || k.startsWith(path + '.'))
    .map(([, v]) => v);
  const source = sources.includes('managed')
    ? 'managed'
    : sources.includes('host')
      ? 'host'
      : 'shipped';
  return <span style={muted}>{source}</span>;
}
function AssignmentEditor({
  name,
  value,
  inherited,
  view,
  onChange,
}: {
  name: string;
  value: RoutingAssignment;
  inherited?: RoutingAssignment;
  view: RoutingPreferencesView;
  onChange: (next: RoutingAssignment) => void;
}) {
  const catalog = view.catalog[value.provider];
  const models = catalog?.models ?? [];
  const current = models.find((m) => m.id === value.model);
  const change = (patch: Partial<RoutingAssignment>) => onChange({ ...value, ...patch });
  return (
    <div style={{ display: 'grid', gap: 8 }}>
      <div style={grid}>
        <Pick
          name={`${name} provider`}
          value={value.provider}
          choices={Object.keys(view.catalog)}
          onChange={(provider) => change({ provider })}
        />
        <Pick
          name={`${name} model`}
          value={value.model}
          choices={models.map((m) => ({ value: m.id, label: m.label || m.id }))}
          onChange={(model) => change({ model })}
        />
        <Pick
          name={`${name} effort`}
          value={value.effort ?? ''}
          choices={[{ value: '', label: 'Provider default' }, ...(current?.effortLevels ?? [])]}
          onChange={(effort) => change({ effort })}
        />
        <Pick
          name={`${name} minimum effort`}
          value={value.minEffort ?? ''}
          choices={[{ value: '', label: 'No additional floor' }, ...(current?.effortLevels ?? [])]}
          onChange={(minEffort) => change({ minEffort })}
        />
      </div>
      <div style={{ display: 'flex', gap: 12, flexWrap: 'wrap', fontSize: '0.72rem' }}>
        <label>
          <input
            type="checkbox"
            checked={value.enabled !== false}
            onChange={(e) => change({ enabled: e.target.checked })}
          />{' '}
          Enabled
        </label>
        <label>
          <input
            type="checkbox"
            checked={value.fresh === true}
            disabled={inherited?.fresh}
            onChange={(e) => change({ fresh: e.target.checked })}
          />{' '}
          Fresh context{inherited?.fresh ? ' (host floor)' : ''}
        </label>
      </div>
      <div style={muted}>
        Catalog: {catalog?.state || 'unknown'}
        {catalog?.observedAt
          ? ` · observed ${new Date(catalog.observedAt).toLocaleTimeString()}`
          : ' · not observed'}
        .{' '}
        {!current?.effortLevels?.length &&
          'Effort ladder not reported; current value and provider default are shown.'}
      </div>
    </div>
  );
}

export default function RoutingSection() {
  const [view, setView] = useState<RoutingPreferencesView | null>(null);
  const [draft, setDraft] = useState<RoutingPolicy | null>(null);
  const [profile, setProfile] = useState('');
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState('Loading routing preferences…');
  const [validation, setValidation] = useState<RoutingValidation | null>(null);
  const [validated, setValidated] = useState(false);
  const [conflict, setConflict] = useState(false);
  const [role, setRole] = useState('scout');
  const [previewProvider, setPreviewProvider] = useState('');
  const [preview, setPreview] = useState<RoutingPreview | null>(null);
  const adopt = (v: RoutingPreferencesView) => {
    setView(v);
    setDraft(clone(v.effective));
    setProfile(v.effective.activeProfile);
    setValidation(v.validation);
    setValidated(false);
    setConflict(false);
    setPreview(null);
  };
  const reload = async () => {
    setBusy(true);
    try {
      const v = await window.electronAPI.routingPreferencesGet();
      if (v.schemaVersion !== 1)
        throw new Error('This hub uses an unsupported routing preferences version');
      adopt(v);
      setMessage(
        v.warning ||
          (v.configurable
            ? 'Preferences belong to this connected hub.'
            : 'Editing unavailable: authenticated host authority is required. Peer editing is unavailable.'),
      );
    } catch (e) {
      setView(null);
      setDraft(null);
      setMessage(
        `Routing unavailable on this connected hub: ${e instanceof Error ? e.message : String(e)}`,
      );
    } finally {
      setBusy(false);
    }
  };
  useEffect(() => {
    void reload();
  }, []);
  const edit = (fn: (next: RoutingPolicy) => void) => {
    if (!draft) return;
    const next = clone(draft);
    fn(next);
    setDraft(next);
    setValidated(false);
    setValidation(null);
    setMessage('Unapplied changes');
  };
  const patch = view && draft ? routingDifference(view.effective, draft) : {};
  const dirty = Object.keys(patch).length > 0;
  const transact = async (action: 'Validate' | 'Apply' | 'Reset') => {
    if (!view) return;
    setBusy(true);
    try {
      const request = { baseRevision: view.revision, patch };
      const result =
        action === 'Reset'
          ? await window.electronAPI.routingPreferencesReset({ baseRevision: view.revision })
          : action === 'Apply'
            ? await window.electronAPI.routingPreferencesSave(request)
            : await window.electronAPI.routingPreferencesValidate(request);
      setValidation(result.validation);
      if (result.status === 'applied') {
        adopt(result.view);
        setMessage(
          action === 'Reset'
            ? 'Managed preferences cleared. Inherited host/shipped values are active.'
            : 'Applied to the live routing service.',
        );
      } else if (result.status === 'valid') {
        setValidated(true);
        setMessage('Validated. Ready to apply.');
      } else if (result.status === 'conflict') {
        setConflict(true);
        setValidated(false);
        setMessage(
          'The hub policy changed. Reload before editing again; this draft has not been applied.',
        );
      } else {
        setValidated(false);
        setMessage(
          result.validation.catalogPending
            ? 'Catalog unknown for a changed assignment. Pending and unapplied; reload after the hub has a current catalog.'
            : `Preferences ${result.status}. Nothing applied.`,
        );
      }
    } catch (e) {
      setValidated(false);
      setMessage(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };
  const tryRoute = async () => {
    setBusy(true);
    try {
      setPreview(
        await window.electronAPI.routingPreview({
          role,
          profile,
          provider: previewProvider || undefined,
        }),
      );
    } catch (e) {
      setMessage(String(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Section title="Routing">
      <div
        role="region"
        aria-label="Routing settings"
        style={{ display: 'grid', gap: 16, minWidth: 0, fontSize: '0.8rem' }}
      >
        <p style={muted}>
          Choose models for routed worker roles. The Fleet Manager’s own model is configured in
          Fleet Manager settings; workflow stages are configured separately. Project safety limits
          are enforced by this hub and are not editable here.
        </p>
        <div>
          <SmallButton onClick={() => openPolicySettings('supervisor')} disabled={busy || dirty}>
            Fleet workflows
          </SmallButton>
        </div>
        <div
          role="status"
          style={{ ...muted, color: conflict ? 'var(--wks-warning)' : 'var(--wks-text-secondary)' }}
        >
          {message}
        </div>
        <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
          <SmallButton onClick={() => void reload()} disabled={busy}>
            {conflict ? 'Reload changed policy' : 'Reload'}
          </SmallButton>
          <SmallButton
            onClick={() => void transact('Validate')}
            disabled={busy || !view?.configurable || !dirty || conflict}
          >
            Validate
          </SmallButton>
          <SmallButton
            onClick={() => void transact('Apply')}
            disabled={busy || !view?.configurable || !dirty || !validated || conflict}
          >
            Apply
          </SmallButton>
          <SmallButton
            onClick={() => void transact('Reset')}
            disabled={busy || !view?.configurable || conflict || !view.managedFields.length}
          >
            Reset managed preferences
          </SmallButton>
        </div>
        {validation?.issues.map((i, n) => (
          <div key={n} role="alert" style={{ ...muted, color: 'var(--wks-error)' }}>
            {i.where}: {i.detail}
          </div>
        ))}
        {view && draft && (
          <>
            <fieldset
              disabled={busy || !view.configurable || conflict}
              style={{ border: 0, padding: 0, margin: 0, minWidth: 0, display: 'grid', gap: 16 }}
            >
              <div>
                <Pick
                  name="Active preset"
                  value={draft.activeProfile}
                  choices={Object.keys(draft.profiles)}
                  onChange={(activeProfile) =>
                    edit((n) => {
                      n.activeProfile = activeProfile;
                    })
                  }
                />
                <Source view={view} path="activeProfile" />{' '}
                <span style={muted}>
                  · Shipped: {view.defaults.activeProfile} · Inherited:{' '}
                  {view.inherited.activeProfile}
                </span>
              </div>
              <h3 style={{ fontSize: '0.9rem', margin: 0 }}>Work roles</h3>
              <div style={grid}>
                {Object.entries(draft.roles)
                  .filter(([r]) => r !== 'supervisor')
                  .map(([r, c]) => (
                    <div key={r}>
                      <Pick
                        name={`${label(r)} capability`}
                        value={c}
                        choices={Object.keys(draft.profiles[draft.activeProfile])}
                        onChange={(cap) =>
                          edit((n) => {
                            n.roles[r] = cap;
                          })
                        }
                      />
                      <div style={muted}>{roleHints[r] || 'Host-defined work role'}</div>
                      <Source view={view} path={`roles.${r}`} />{' '}
                      <span style={muted}>
                        · Shipped: {view.defaults.roles[r] ?? 'host-defined'} · Inherited:{' '}
                        {view.inherited.roles[r]}
                      </span>
                    </div>
                  ))}
              </div>
              <h3 style={{ fontSize: '0.9rem', margin: 0 }}>Model assignments</h3>
              <Pick
                name="Assignment profile"
                value={profile}
                choices={Object.keys(draft.profiles)}
                onChange={setProfile}
              />
              {Object.entries(draft.profiles[profile] ?? {}).map(([cap, a]) => (
                <Surface key={`${profile}.${cap}`} pad="md">
                  <div style={{ display: 'grid', gap: 12 }}>
                    <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
                      <strong>{label(cap)}</strong>
                      <Source view={view} path={`profiles.${profile}.${cap}`} />
                    </div>
                    <AssignmentEditor
                      name={cap}
                      value={a}
                      inherited={view.inherited.profiles[profile]?.[cap]}
                      view={view}
                      onChange={(value) =>
                        edit((n) => {
                          n.profiles[profile][cap] = value;
                        })
                      }
                    />
                    <details>
                      <summary style={{ cursor: 'pointer', fontSize: '0.72rem' }}>
                        Defaults and inherited assignment
                      </summary>
                      {(['defaults', 'inherited'] as const).map((source) => {
                        const value = view[source].profiles[profile]?.[cap];
                        return (
                          <p key={source} style={muted}>
                            {source === 'defaults' ? 'Shipped' : 'Inherited'}:{' '}
                            {value
                              ? `${value.provider} / ${value.model} / ${value.effort || 'provider default'} · minimum ${value.minEffort || 'none'} · ${value.fresh ? 'fresh context' : 'continuation allowed'}`
                              : 'Defined by host'}
                          </p>
                        );
                      })}
                    </details>
                    <details>
                      <summary style={{ cursor: 'pointer', fontSize: '0.72rem' }}>
                        Ordered alternatives ({a.alternatives?.length ?? 0})
                      </summary>
                      <p style={muted}>
                        Fallover tries these in order when the primary cannot be used. This differs
                        from a mode shift, which changes the capability or effort.
                      </p>
                      {(a.alternatives ?? []).map((alt, i) => (
                        <div key={i} style={{ display: 'grid', gap: 8, marginBlock: 12 }}>
                          <AssignmentEditor
                            name={`${cap} alternative ${i + 1}`}
                            value={alt}
                            inherited={view.inherited.profiles[profile]?.[cap]}
                            view={view}
                            onChange={(value) =>
                              edit((n) => {
                                n.profiles[profile][cap].alternatives![i] = value;
                              })
                            }
                          />
                          <div style={{ display: 'flex', gap: 8 }}>
                            <SmallButton
                              disabled={i === 0}
                              onClick={() =>
                                edit((n) => {
                                  const list = n.profiles[profile][cap].alternatives!;
                                  [list[i - 1], list[i]] = [list[i], list[i - 1]];
                                })
                              }
                            >
                              Move up
                            </SmallButton>
                            <SmallButton
                              onClick={() =>
                                edit((n) => {
                                  n.profiles[profile][cap].alternatives!.splice(i, 1);
                                })
                              }
                            >
                              Remove
                            </SmallButton>
                          </div>
                        </div>
                      ))}
                      <SmallButton
                        onClick={() =>
                          edit((n) => {
                            const { alternatives: _, ...primary } = a;
                            n.profiles[profile][cap].alternatives = [
                              ...(a.alternatives ?? []),
                              primary,
                            ];
                          })
                        }
                      >
                        Add alternative
                      </SmallButton>
                    </details>
                  </div>
                </Surface>
              ))}
              <details>
                <summary style={{ cursor: 'pointer' }}>Advanced policy</summary>
                <div style={{ display: 'grid', gap: 16, marginTop: 12 }}>
                  <Pick
                    name="Global mode"
                    value={draft.modes.global}
                    choices={modes}
                    onChange={(v) =>
                      edit((n) => {
                        n.modes.global = v;
                      })
                    }
                  />
                  <div style={grid}>
                    {Object.entries(draft.providers).map(([p, provider]) => (
                      <div key={p}>
                        <label>
                          <input
                            type="checkbox"
                            checked={provider.enabled !== false}
                            onChange={(e) =>
                              edit((n) => {
                                n.providers[p].enabled = e.target.checked;
                              })
                            }
                          />{' '}
                          {p} enabled
                        </label>
                        <Pick
                          name={`${p} mode`}
                          value={draft.modes.providers[p] || 'auto'}
                          choices={modes}
                          onChange={(v) =>
                            edit((n) => {
                              n.modes.providers[p] = v;
                            })
                          }
                        />
                      </div>
                    ))}
                  </div>
                  <PolicyControls
                    value={draft.thresholds}
                    prefix="thresholds"
                    view={view}
                    update={(path, value) => edit((n) => setPolicyValue(n, path, value))}
                  />
                  <PolicyControls
                    value={draft.forecastWeights}
                    prefix="forecastWeights"
                    view={view}
                    update={(path, value) => edit((n) => setPolicyValue(n, path, value))}
                  />
                  {Object.entries(draft.modeShifts).map(([mode, shift]) => (
                    <div key={mode} style={{ display: 'grid', gap: 8 }}>
                      <strong>{label(mode)} shifts</strong>
                      <label style={muted}>
                        Effort step
                        <input
                          aria-label={`${mode} effort step`}
                          type="number"
                          min={-5}
                          max={5}
                          step={1}
                          style={selectStyle}
                          value={shift.effortStep ?? 0}
                          onChange={(e) => {
                            if (Number.isFinite(e.target.valueAsNumber))
                              edit((n) => {
                                n.modeShifts[mode].effortStep = e.target.valueAsNumber;
                              });
                          }}
                        />
                      </label>
                      <div style={grid}>
                        {Object.entries(shift.roles ?? {}).map(([r, c]) => (
                          <Pick
                            key={r}
                            name={`${mode} ${r}`}
                            value={c}
                            choices={Object.keys(draft.profiles[profile])}
                            onChange={(v) =>
                              edit((n) => {
                                n.modeShifts[mode].roles![r] = v;
                              })
                            }
                          />
                        ))}
                      </div>
                      <div style={muted}>
                        Capabilities whose effort may step (none selected means all):
                      </div>
                      <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
                        {Object.keys(draft.profiles[profile]).map((c) => (
                          <label key={c} style={muted}>
                            <input
                              type="checkbox"
                              checked={shift.effortStepCapabilities?.includes(c) ?? false}
                              onChange={(e) =>
                                edit((n) => {
                                  const current = n.modeShifts[mode].effortStepCapabilities ?? [];
                                  n.modeShifts[mode].effortStepCapabilities = e.target.checked
                                    ? [...current, c]
                                    : current.filter((v) => v !== c);
                                })
                              }
                            />
                            {label(c)}
                          </label>
                        ))}
                      </div>
                    </div>
                  ))}
                </div>
              </details>
            </fieldset>
            <details>
              <summary style={{ cursor: 'pointer' }}>Try this route</summary>
              <p style={muted}>
                Uses the applied policy and a bounded usage snapshot. Preview never launches an
                agent or records a routing decision. Apply your draft first to preview its effect.
              </p>
              <div style={grid}>
                <Pick
                  name="Preview role"
                  value={role}
                  choices={Object.keys(draft.roles)}
                  onChange={setRole}
                />
                <Pick
                  name="Preview provider"
                  value={previewProvider}
                  choices={[{ value: '', label: 'Policy chooses' }, ...Object.keys(view.catalog)]}
                  onChange={setPreviewProvider}
                />
              </div>
              <SmallButton disabled={busy} onClick={() => void tryRoute()}>
                Preview route
              </SmallButton>
              {preview && (
                <div aria-label="Route preview">
                  <p>
                    {preview.eligible
                      ? `${preview.provider} / ${preview.model} / ${preview.effort || 'provider default'}`
                      : 'No eligible route'}{' '}
                    · {preview.mode} · {preview.capability}
                  </p>
                  <div style={muted}>
                    {preview.usageState} at {new Date(preview.observedAt).toLocaleTimeString()}
                    {preview.fresh ? ' · Fresh context required' : ''}
                  </div>
                  <ul style={{ ...muted, paddingLeft: 20 }}>
                    {preview.reason.map((r, i) => (
                      <li key={i}>{r}</li>
                    ))}
                  </ul>
                </div>
              )}
            </details>
          </>
        )}
      </div>
    </Section>
  );
}

function setPolicyValue(policy: RoutingPolicy, path: string, value: string | number | boolean) {
  const parts = path.split('.');
  let target = policy as unknown as Record<string, unknown>;
  for (const part of parts.slice(0, -1)) target = target[part] as Record<string, unknown>;
  target[parts.at(-1)!] = value;
}
const enums: Record<string, string[]> = {
  curve: ['calendar', 'workdays', 'five_day'],
  weekend: ['spend_tail', 'reserve'],
};
function PolicyControls({
  value,
  prefix,
  view,
  update,
}: {
  value: object;
  prefix: string;
  view: RoutingPreferencesView;
  update: (path: string, value: string | number | boolean) => void;
}) {
  return (
    <div style={grid}>
      {Object.entries(value).map(([key, v]) => {
        const path = `${prefix}.${key}`;
        if (v !== null && typeof v === 'object')
          return (
            <div key={key} style={{ gridColumn: '1 / -1' }}>
              <h4 style={{ fontSize: '0.8rem' }}>{label(key)}</h4>
              <PolicyControls value={v} prefix={path} view={view} update={update} />
            </div>
          );
        return (
          <div key={key}>
            {enums[key] ? (
              <Pick
                name={label(key)}
                value={String(v)}
                choices={enums[key]}
                onChange={(next) => update(path, next)}
              />
            ) : (
              <label style={{ display: 'grid', gap: 4, fontSize: '0.72rem' }}>
                {label(key)}
                <input
                  aria-label={path}
                  style={typeof v === 'boolean' ? undefined : selectStyle}
                  type={
                    typeof v === 'boolean' ? 'checkbox' : typeof v === 'number' ? 'number' : 'text'
                  }
                  checked={typeof v === 'boolean' ? v : undefined}
                  value={typeof v === 'boolean' ? undefined : v}
                  step={typeof v === 'number' ? 'any' : undefined}
                  onChange={(e) => {
                    if (typeof v === 'number' && !Number.isFinite(e.target.valueAsNumber)) return;
                    update(
                      path,
                      typeof v === 'boolean'
                        ? e.target.checked
                        : typeof v === 'number'
                          ? e.target.valueAsNumber
                          : e.target.value,
                    );
                  }}
                />
              </label>
            )}
            <Source view={view} path={path} />
          </div>
        );
      })}
    </div>
  );
}
