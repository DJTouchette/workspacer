import React, { useState, useMemo } from 'react';
import { useLibrary } from '../hooks/useLibrary';
import { runLibraryItem } from '../lib/libraryBus';
import MarkdownEditor from '../components/MarkdownEditor';
import { Zap } from '../components/icons';
import type { LibraryItem, LibraryKind, LibraryScope, LibraryAction, LibrarySaveInput } from '../types/library';

interface Props {
  title?: string;
  /** Project root for project-scoped items (falls back to the app cwd). */
  cwd?: string;
}

type Draft = {
  original?: LibraryItem;
  scope: LibraryScope;
  title: string;
  kind: LibraryKind;
  description: string;
  tags: string;
  action: LibraryAction;
  body: string;
};

const blankDraft = (): Draft => ({
  scope: 'global', title: '', kind: 'prompt', description: '', tags: '', action: 'insert', body: '',
});

const LibraryPane: React.FC<Props> = ({ cwd }) => {
  const { items, save, remove } = useLibrary(cwd);
  const [query, setQuery] = useState('');
  const [scopeFilter, setScopeFilter] = useState<'all' | LibraryScope>('all');
  const [draft, setDraft] = useState<Draft | null>(null);

  const filtered = useMemo(() => {
    const q = query.toLowerCase();
    return items.filter((it) =>
      (scopeFilter === 'all' || it.scope === scopeFilter) &&
      (q === '' ||
        it.title.toLowerCase().includes(q) ||
        (it.description ?? '').toLowerCase().includes(q) ||
        (it.tags ?? []).some((t) => t.toLowerCase().includes(q)) ||
        it.body.toLowerCase().includes(q)),
    );
  }, [items, query, scopeFilter]);

  const startEdit = (it: LibraryItem) => setDraft({
    original: it, scope: it.scope, title: it.title, kind: it.kind,
    description: it.description ?? '', tags: (it.tags ?? []).join(', '),
    action: it.action ?? 'insert', body: it.body,
  });

  const saveDraft = async () => {
    if (!draft || !draft.title.trim()) return;
    const isClaude = draft.scope === 'claude';
    const input: LibrarySaveInput = {
      scope: draft.scope,
      id: draft.original?.id,
      title: draft.title.trim(),
      kind: draft.kind,
      // tags/action aren't part of Claude Code's frontmatter format
      description: draft.description.trim() || undefined,
      tags: isClaude ? undefined : draft.tags.split(',').map((t) => t.trim()).filter(Boolean),
      action: isClaude ? undefined : draft.action,
      body: draft.body,
      cwd,
    };
    // If the storage location changed (scope, or kind within claude scope —
    // skills and agents live in different dirs), remove the old file first.
    if (draft.original && (
      draft.original.scope !== draft.scope ||
      (draft.scope === 'claude' && draft.original.kind !== draft.kind)
    )) {
      await remove(draft.original.scope, draft.original.id, draft.original.kind);
    }
    await save(input);
    setDraft(null);
  };

  // ── Editor view ──
  if (draft) {
    return (
      <Shell>
        <div style={headerStyle}>
          <button onClick={() => setDraft(null)} style={btn()}>← Back</button>
          <div style={{ flex: 1 }} />
          <button onClick={saveDraft} disabled={!draft.title.trim()} style={btn(true)}>Save</button>
        </div>
        <div style={{ padding: 16, overflow: 'auto' }}>
          <Field label="Title">
            <input value={draft.title} onChange={(e) => setDraft({ ...draft, title: e.target.value })} style={inputStyle} placeholder="Refactor for testability" />
          </Field>
          <div style={{ display: 'grid', gridTemplateColumns: draft.scope === 'claude' ? '1fr 1fr' : '1fr 1fr 1fr', gap: 10 }}>
            <Field label="Kind">
              <select value={draft.kind} onChange={(e) => setDraft({ ...draft, kind: e.target.value as LibraryKind })} style={inputStyle}>
                {draft.scope !== 'claude' && <option value="prompt">prompt</option>}
                <option value="skill">skill</option>
                <option value="agent">agent</option>
              </select>
            </Field>
            <Field label="Scope">
              <select
                value={draft.scope}
                onChange={(e) => {
                  const scope = e.target.value as LibraryScope;
                  // .claude only stores skills and agents, never prompts
                  const kind = scope === 'claude' && draft.kind === 'prompt' ? 'skill' : draft.kind;
                  setDraft({ ...draft, scope, kind });
                }}
                style={inputStyle}
              >
                <option value="global">global</option>
                <option value="project">project</option>
                <option value="claude">claude (.claude/)</option>
              </select>
            </Field>
            {draft.scope !== 'claude' && (
              <Field label="Default action">
                <select value={draft.action} onChange={(e) => setDraft({ ...draft, action: e.target.value as LibraryAction })} style={inputStyle}>
                  <option value="insert">insert</option>
                  <option value="spawn">spawn</option>
                  <option value="copy">copy</option>
                </select>
              </Field>
            )}
          </div>
          <Field label="Description">
            <input value={draft.description} onChange={(e) => setDraft({ ...draft, description: e.target.value })} style={inputStyle} placeholder="Short summary (shown in lists)" />
          </Field>
          {draft.scope !== 'claude' && (
            <Field label="Tags (comma-separated)">
              <input value={draft.tags} onChange={(e) => setDraft({ ...draft, tags: e.target.value })} style={inputStyle} placeholder="refactor, tests" />
            </Field>
          )}
          <Field label="Body — supports {{cwd}}, {{selection}}, {{clipboard}} + form fields">
            <MarkdownEditor
              value={draft.body}
              onChange={(body) => setDraft({ ...draft, body })}
              placeholder="The prompt or skill text…"
            />
            <FormFieldLegend />
          </Field>
          {draft.scope === 'project' && (
            <div style={{ fontSize: '0.62rem', color: 'var(--wks-text-faint)', marginTop: 4 }}>
              Project items save to <code>{cwd ? `${cwd}/.workspacer/library/` : '.workspacer/library/'}</code>
            </div>
          )}
          {draft.scope === 'claude' && (
            <div style={{ fontSize: '0.62rem', color: 'var(--wks-text-faint)', marginTop: 4 }}>
              Saves to <code>{`${cwd ?? '.'}/${draft.kind === 'agent' ? '.claude/agents/<id>.md' : '.claude/skills/<id>/SKILL.md'}`}</code> in
              Claude Code's native format — extra frontmatter (tools, model, ...) is preserved.
            </div>
          )}
        </div>
      </Shell>
    );
  }

  // ── List view ──
  return (
    <Shell>
      <div style={headerStyle}>
        <span style={{ display: 'flex', alignItems: 'center', gap: 6, fontWeight: 600, fontSize: '0.8rem' }}>
          <Zap size={15} strokeWidth={1.9} /> Library
        </span>
        <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Search prompts & skills…" spellCheck={false}
          style={{ ...inputStyle, height: 28, flex: 1, maxWidth: 280, margin: '0 8px' }} />
        <div style={{ display: 'flex', gap: 2, marginRight: 8 }}>
          {(['all', 'global', 'project', 'claude'] as const).map((s) => (
            <button key={s} onClick={() => setScopeFilter(s)} style={chip(scopeFilter === s)}>{s}</button>
          ))}
        </div>
        <button onClick={() => setDraft(blankDraft())} style={btn(true)}>+ New</button>
      </div>

      <div style={{ overflow: 'auto', padding: 12 }}>
        {filtered.length === 0 && (
          <div style={{ textAlign: 'center', color: 'var(--wks-text-faint)', padding: 40, fontSize: '0.78rem' }}>
            {items.length === 0 ? 'No items yet. Click + New to create one.' : 'No matches.'}
          </div>
        )}
        {filtered.map((it) => (
          <div key={`${it.scope}:${it.id}`} style={cardStyle}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
              <span style={{ fontWeight: 600, fontSize: '0.8rem', color: 'var(--wks-text-primary)' }}>{it.title}</span>
              <span style={kindBadge(it.kind)}>{it.kind}</span>
              <span style={scopeBadge(it.scope)}>{it.scope}</span>
              <div style={{ flex: 1 }} />
              <button onClick={() => runLibraryItem(it, 'insert')} style={miniBtn} title="Insert into focused agent">Insert</button>
              <button onClick={() => runLibraryItem(it, 'spawn')} style={miniBtn} title="Spawn a new agent with this">Spawn</button>
              <button onClick={() => runLibraryItem(it, 'copy')} style={miniBtn} title="Copy to clipboard">Copy</button>
              <button onClick={() => startEdit(it)} style={miniBtn}>Edit</button>
              <button
                onClick={() => {
                  const warning = it.scope === 'claude' && it.kind === 'skill'
                    ? `Delete “${it.title}”? This removes the whole .claude/skills/${it.id}/ folder (including any resource files).`
                    : `Delete “${it.title}”?`;
                  if (confirm(warning)) remove(it.scope, it.id, it.kind);
                }}
                style={{ ...miniBtn, color: 'var(--wks-danger, #ff8a8a)' }}
              >Delete</button>
            </div>
            {it.description && <div style={{ fontSize: '0.68rem', color: 'var(--wks-text-secondary)', marginTop: 3 }}>{it.description}</div>}
            <div style={{ fontSize: '0.62rem', color: 'var(--wks-text-faint)', marginTop: 4, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis', fontFamily: 'var(--wks-mono, ui-monospace, monospace)' }}>
              {it.body.replace(/\s+/g, ' ').slice(0, 120)}
            </div>
            {it.tags && it.tags.length > 0 && (
              <div style={{ display: 'flex', gap: 4, marginTop: 5, flexWrap: 'wrap' }}>
                {it.tags.map((t) => <span key={t} style={tagStyle}>{t}</span>)}
              </div>
            )}
          </div>
        ))}
      </div>
    </Shell>
  );
};

// ── small presentational helpers ──

const Shell: React.FC<{ children: React.ReactNode }> = ({ children }) => (
  <div style={{ height: '100%', display: 'flex', flexDirection: 'column', background: 'var(--wks-bg-base)', color: 'var(--wks-text-primary)' }}>{children}</div>
);

const Field: React.FC<{ label: string; children: React.ReactNode }> = ({ label, children }) => (
  <div style={{ marginBottom: 12 }}>
    <div style={{ fontSize: '0.62rem', textTransform: 'uppercase', letterSpacing: '0.04em', color: 'var(--wks-text-disabled)', marginBottom: 4 }}>{label}</div>
    {children}
  </div>
);

/** Cheat-sheet for the {{?…}} form-field syntax, shown under the body editor.
 *  When an item with these tokens runs, a form pops up to collect the values. */
const FormFieldLegend: React.FC = () => {
  const rows: Array<[string, string]> = [
    ['{{?Context}}', 'paragraph (multi-line) — the default'],
    ['{{?Service|text}}', 'single-line text'],
    ['{{?Env|select:dev,staging,prod}}', 'dropdown (first option is the default)'],
    ['{{?Verbose|toggle:--verbose,}}', 'checkbox → injects the on/off value'],
  ];
  return (
    <div style={{ marginTop: 6, padding: '8px 10px', borderRadius: 6, background: 'var(--wks-bg-base)', border: '1px solid var(--wks-border-subtle)' }}>
      <div style={{ fontSize: '0.6rem', color: 'var(--wks-text-faint)', marginBottom: 5 }}>
        Form fields — a form pops up to collect these when the item runs. Add a default with a colon, e.g. <code style={codeStyle}>{'{{?Service:payments-api|text}}'}</code>.
      </div>
      {rows.map(([tok, desc]) => (
        <div key={tok} style={{ display: 'flex', gap: 8, fontSize: '0.6rem', lineHeight: 1.7 }}>
          <code style={{ ...codeStyle, flexShrink: 0 }}>{tok}</code>
          <span style={{ color: 'var(--wks-text-faint)' }}>{desc}</span>
        </div>
      ))}
    </div>
  );
};

const codeStyle: React.CSSProperties = {
  fontFamily: 'var(--wks-mono, ui-monospace, monospace)', fontSize: '0.58rem',
  color: 'var(--wks-text-secondary)', background: 'var(--wks-bg-selected)',
  padding: '1px 5px', borderRadius: 4,
};

const headerStyle: React.CSSProperties = {
  display: 'flex', alignItems: 'center', gap: 6, padding: '8px 12px',
  borderBottom: '1px solid var(--wks-border)', flexShrink: 0, background: 'var(--wks-bg-raised)',
};
const inputStyle: React.CSSProperties = {
  width: '100%', fontFamily: 'inherit', fontSize: '0.75rem', padding: '6px 8px', borderRadius: 5, outline: 'none',
  background: 'var(--wks-bg-input)', color: 'var(--wks-text-primary)', border: '1px solid var(--wks-border-input)', boxSizing: 'border-box',
};
const cardStyle: React.CSSProperties = {
  border: '1px solid var(--wks-border)', borderRadius: 8, padding: '8px 10px', marginBottom: 8, background: 'var(--wks-bg-raised)',
};
const tagStyle: React.CSSProperties = {
  fontSize: '0.58rem', padding: '1px 6px', borderRadius: 999, background: 'var(--wks-bg-selected)', color: 'var(--wks-text-secondary)',
};
const miniBtn: React.CSSProperties = {
  fontSize: '0.66rem', fontFamily: 'inherit', cursor: 'pointer', padding: '3px 8px', borderRadius: 4,
  border: '1px solid var(--wks-border-input)', background: 'transparent', color: 'var(--wks-text-secondary)',
};
function btn(primary = false): React.CSSProperties {
  return {
    fontSize: '0.72rem', fontFamily: 'inherit', fontWeight: 600, cursor: 'pointer', padding: '5px 12px', borderRadius: 5,
    border: primary ? 'none' : '1px solid var(--wks-border-input)',
    background: primary ? 'var(--wks-accent)' : 'transparent',
    color: primary ? 'var(--wks-text-on-accent, #fff)' : 'var(--wks-text-secondary)',
  };
}
function chip(active: boolean): React.CSSProperties {
  return {
    fontSize: '0.62rem', fontFamily: 'inherit', cursor: 'pointer', padding: '3px 8px', borderRadius: 4,
    border: '1px solid ' + (active ? 'var(--wks-accent)' : 'var(--wks-border-input)'),
    background: active ? 'var(--wks-accent)' : 'transparent',
    color: active ? 'var(--wks-text-on-accent, #fff)' : 'var(--wks-text-secondary)',
  };
}
function kindBadge(kind: LibraryKind): React.CSSProperties {
  const palette: Record<LibraryKind, { bg: string; fg: string }> = {
    prompt: { bg: 'rgba(96,165,250,0.18)', fg: '#60a5fa' },
    skill: { bg: 'rgba(192,132,252,0.18)', fg: '#c084fc' },
    agent: { bg: 'rgba(74,222,128,0.18)', fg: '#4ade80' },
  };
  const { bg, fg } = palette[kind] ?? palette.prompt;
  return { fontSize: '0.55rem', padding: '1px 6px', borderRadius: 999, fontWeight: 700, textTransform: 'uppercase',
    background: bg, color: fg };
}
function scopeBadge(scope: LibraryScope): React.CSSProperties {
  // claude-scoped items get a distinct tint — they live in .claude/, not the library dirs
  if (scope === 'claude') {
    return { fontSize: '0.55rem', padding: '1px 6px', borderRadius: 999, fontWeight: 600,
      background: 'rgba(251,146,60,0.16)', color: '#fb923c' };
  }
  return { fontSize: '0.55rem', padding: '1px 6px', borderRadius: 999, fontWeight: 600,
    background: 'var(--wks-bg-selected)', color: 'var(--wks-text-faint)' };
}

export default LibraryPane;
