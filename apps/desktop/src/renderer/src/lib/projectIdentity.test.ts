import { describe, it, expect } from 'vitest';
import { resolveProject, initialsOf, basenameOf } from './projectIdentity';

describe('basenameOf', () => {
  it('is the last segment, whatever the separator or trailing slash', () => {
    expect(basenameOf('/home/me/work/api-gateway')).toBe('api-gateway');
    expect(basenameOf('/home/me/work/api-gateway/')).toBe('api-gateway');
    expect(basenameOf('C:\\Users\\me\\repo')).toBe('repo');
    expect(basenameOf('')).toBe('');
  });
});

describe('initialsOf', () => {
  it('takes word initials, so sibling repos sharing a prefix stay distinct', () => {
    // The whole reason this is not just "first two letters": these two live
    // next to each other in the fleet and must not both read AP.
    expect(initialsOf('api-gateway')).toBe('AG');
    expect(initialsOf('api-worker')).toBe('AW');
    expect(initialsOf('work_spacer')).toBe('WS');
    expect(initialsOf('my project')).toBe('MP');
  });

  it('splits a camelCase single word rather than doubling its first letters', () => {
    expect(initialsOf('workSpacer')).toBe('WS');
  });

  it('falls back to the first two letters, then to a placeholder', () => {
    expect(initialsOf('claudemon')).toBe('CL');
    expect(initialsOf('')).toBe('?');
    expect(initialsOf('---')).toBe('?');
  });
});

describe('resolveProject', () => {
  it('works with NO configuration — that is the point', () => {
    // The fleet has to be legible before anyone opens a settings page.
    const p = resolveProject('/home/me/work/api-gateway')!;
    expect(p.label).toBe('api-gateway');
    expect(p.initials).toBe('AG');
    expect(p.color).toMatch(/^#[0-9a-f]{6}$/i);
    expect(p.icon).toBeUndefined();
  });

  it('derives a STABLE colour from the path', () => {
    const a = resolveProject('/home/me/work/api-gateway')!;
    const b = resolveProject('/home/me/work/api-gateway')!;
    expect(a.color).toBe(b.color);
    // Different projects should generally differ; pin that the hash actually
    // spreads rather than collapsing everything onto one entry.
    const colors = new Set(
      ['alpha', 'beta', 'gamma', 'delta', 'epsilon', 'zeta'].map(
        (n) => resolveProject('/w/' + n)!.color,
      ),
    );
    expect(colors.size).toBeGreaterThan(3);
  });

  it('lets config override each part independently', () => {
    const cfg = { '/w/repo': { label: 'Platform API', icon: '🚀', color: '#ff0000' } };
    const p = resolveProject('/w/repo', cfg)!;
    expect(p.label).toBe('Platform API');
    expect(p.icon).toBe('🚀');
    expect(p.color).toBe('#ff0000');
    // Initials follow the LABEL, so renaming renames the mark.
    expect(p.initials).toBe('PA');
  });

  it('keys the same directory the way scripts and widgets do', () => {
    // Trailing slashes and backslashes are a spelling accident, not a
    // different project — projectKey normalizes both.
    const cfg = { '/w/repo': { label: 'Kept' } };
    expect(resolveProject('/w/repo/', cfg)!.label).toBe('Kept');
    expect(resolveProject('\\w\\repo', cfg)!.label).toBe('Kept');
  });

  it('colours by path, not by label — renaming must not recolour', () => {
    const plain = resolveProject('/w/repo')!;
    const renamed = resolveProject('/w/repo', { '/w/repo': { label: 'Something Else' } })!;
    expect(renamed.color).toBe(plain.color);
  });

  it('treats blank config values as unset rather than as an override', () => {
    const p = resolveProject('/w/repo', { '/w/repo': { label: '  ', icon: '', color: '' } })!;
    expect(p.label).toBe('repo');
    expect(p.icon).toBeUndefined();
    expect(p.color).toMatch(/^#[0-9a-f]{6}$/i);
  });

  it('renders a DOWNLOADED icon over the source URL', () => {
    // The URL is provenance; the cached file is what draws, so the mark keeps
    // working offline and never re-requests on render.
    const p = resolveProject('/w/repo', {
      '/w/repo': { favicon: 'https://x/icon.png', iconFile: 'abc123.png' },
    })!;
    expect(p.iconSrc).toBe('workspacer-icon://abc123.png');
    expect(p.favicon).toBe('https://x/icon.png');
  });

  it('still renders a bare URL, so a config written before caching keeps working', () => {
    const p = resolveProject('/w/repo', { '/w/repo': { favicon: 'https://x/icon.png' } })!;
    expect(p.iconSrc).toBe('https://x/icon.png');
  });

  it('encodes the cached filename into the protocol URL', () => {
    const p = resolveProject('/w/repo', { '/w/repo': { iconFile: 'a b.png' } })!;
    expect(p.iconSrc).toBe('workspacer-icon://a%20b.png');
  });

  it('returns null without a directory', () => {
    expect(resolveProject(undefined)).toBeNull();
    expect(resolveProject('')).toBeNull();
  });
});
