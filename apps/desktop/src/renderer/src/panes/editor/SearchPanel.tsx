/**
 * SearchPanel — project-wide ripgrep search UI living inside the EditorPane
 * sidebar. Self-contained: it calls window.electronAPI.searchProject and, on a
 * result click, hands the (file, line) back to the pane via onOpenMatch. Styling
 * mirrors the file tree (--wks-* tokens, monospace, tiny font).
 */
import React, { useState } from 'react';

interface Match { line: number; column: number; text: string }
interface FileResult { file: string; matches: Match[] }

function basename(p: string): string {
  return p.split(/[\\/]/).pop() || p;
}

type Toggle = 'caseSensitive' | 'wholeWord' | 'regex';

const tabBtnStyle = (on: boolean): React.CSSProperties => ({
  display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
  width: 20, height: 18, cursor: 'pointer', borderRadius: 3,
  border: '1px solid ' + (on ? 'var(--wks-accent, #e6c200)' : 'var(--wks-border-subtle)'),
  background: on ? 'var(--wks-accent-bg)' : 'transparent',
  color: on ? 'var(--wks-text-primary)' : 'var(--wks-text-muted)',
  fontSize: '0.6rem', userSelect: 'none',
});

const SearchPanel: React.FC<{
  cwd: string;
  onOpenMatch: (file: string, line: number) => void;
}> = ({ cwd, onOpenMatch }) => {
  const [query, setQuery] = useState('');
  const [caseSensitive, setCaseSensitive] = useState(false);
  const [wholeWord, setWholeWord] = useState(false);
  const [regex, setRegex] = useState(false);
  const [results, setResults] = useState<FileResult[] | null>(null);
  const [truncated, setTruncated] = useState(false);
  const [searching, setSearching] = useState(false);
  const [error, setError] = useState('');
  // Files whose match list is collapsed (default = expanded).
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});

  const toggles: Record<Toggle, [boolean, React.Dispatch<React.SetStateAction<boolean>>, string, string]> = {
    caseSensitive: [caseSensitive, setCaseSensitive, 'Aa', 'Match case'],
    wholeWord: [wholeWord, setWholeWord, 'W', 'Whole word'],
    regex: [regex, setRegex, '.*', 'Use regular expression'],
  };

  const runSearch = () => {
    const q = query.trim();
    if (!q) { setResults(null); setTruncated(false); setError(''); return; }
    setSearching(true);
    setError('');
    window.electronAPI
      .searchProject({ query: q, cwd, caseSensitive, wholeWord, regex })
      .then((r) => { setResults(r.results); setTruncated(r.truncated); setCollapsed({}); })
      .catch((err: unknown) => { setError(String((err as Error)?.message ?? err)); setResults([]); })
      .finally(() => setSearching(false));
  };

  const totalMatches = results?.reduce((n, f) => n + f.matches.length, 0) ?? 0;

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%', minHeight: 0 }}>
      <div style={{ padding: '6px 8px', display: 'flex', flexDirection: 'column', gap: 6, flex: '0 0 auto' }}>
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => { if (e.key === 'Enter') { e.preventDefault(); runSearch(); } }}
          placeholder="Search project…"
          spellCheck={false}
          style={{
            width: '100%', boxSizing: 'border-box', padding: '3px 6px',
            background: 'var(--wks-bg-base)', color: 'var(--wks-text-primary)',
            border: '1px solid var(--wks-border-subtle)', borderRadius: 3,
            fontFamily: 'inherit', fontSize: '0.7rem', outline: 'none',
          }}
        />
        <div style={{ display: 'flex', gap: 4 }}>
          {(Object.keys(toggles) as Toggle[]).map((k) => {
            const [on, set, label, title] = toggles[k];
            return (
              <span
                key={k}
                title={title}
                onClick={() => { set((v) => !v); }}
                style={tabBtnStyle(on)}
              >
                {label}
              </span>
            );
          })}
          <div style={{ flex: 1 }} />
          <span
            onClick={runSearch}
            title="Search (Enter)"
            style={{ ...tabBtnStyle(false), width: 'auto', padding: '0 6px' }}
          >
            Go
          </span>
        </div>
      </div>

      <div style={{ flex: 1, minHeight: 0, overflow: 'auto', paddingBottom: 8 }}>
        {searching && (
          <div style={{ padding: '4px 8px', color: 'var(--wks-text-disabled)', fontStyle: 'italic' }}>Searching…</div>
        )}
        {!searching && error && (
          <div style={{ padding: '4px 8px', color: 'var(--wks-text-muted)' }}>{error}</div>
        )}
        {!searching && !error && results !== null && results.length === 0 && (
          <div style={{ padding: '4px 8px', color: 'var(--wks-text-disabled)', fontStyle: 'italic' }}>No matches</div>
        )}
        {!searching && !error && results !== null && results.length > 0 && (
          <>
            <div style={{ padding: '2px 8px', color: 'var(--wks-text-disabled)', fontSize: '0.55rem' }}>
              {totalMatches} match{totalMatches === 1 ? '' : 'es'} in {results.length} file{results.length === 1 ? '' : 's'}
              {truncated && ' (truncated)'}
            </div>
            {results.map((f) => {
              const isCollapsed = !!collapsed[f.file];
              return (
                <div key={f.file}>
                  <div
                    onClick={() => setCollapsed((c) => ({ ...c, [f.file]: !c[f.file] }))}
                    title={f.file}
                    style={{
                      display: 'flex', alignItems: 'center', gap: 4, cursor: 'pointer',
                      padding: '1px 6px', whiteSpace: 'nowrap', overflow: 'hidden',
                      textOverflow: 'ellipsis', color: 'var(--wks-text-secondary)',
                    }}
                    onMouseEnter={(e) => (e.currentTarget.style.background = 'var(--wks-bg-hover)')}
                    onMouseLeave={(e) => (e.currentTarget.style.background = 'transparent')}
                  >
                    <span style={{ width: 10, display: 'inline-block', color: 'var(--wks-text-disabled)' }}>{isCollapsed ? '▸' : '▾'}</span>
                    <span style={{ overflow: 'hidden', textOverflow: 'ellipsis' }}>{basename(f.file)}</span>
                    <span style={{ color: 'var(--wks-text-disabled)', fontSize: '0.55rem' }}>{f.matches.length}</span>
                  </div>
                  {!isCollapsed && f.matches.map((m, i) => (
                    <div
                      key={i}
                      onClick={() => onOpenMatch(f.file, m.line)}
                      title={m.text}
                      style={{
                        display: 'flex', gap: 6, cursor: 'pointer',
                        padding: '1px 6px', paddingLeft: 20, whiteSpace: 'nowrap',
                        overflow: 'hidden', textOverflow: 'ellipsis', color: 'var(--wks-text-muted)',
                      }}
                      onMouseEnter={(e) => (e.currentTarget.style.background = 'var(--wks-bg-hover)')}
                      onMouseLeave={(e) => (e.currentTarget.style.background = 'transparent')}
                    >
                      <span style={{ color: 'var(--wks-text-disabled)', flex: '0 0 auto' }}>{m.line}</span>
                      <span style={{ overflow: 'hidden', textOverflow: 'ellipsis' }}>{m.text}</span>
                    </div>
                  ))}
                </div>
              );
            })}
          </>
        )}
      </div>
    </div>
  );
};

export default SearchPanel;
