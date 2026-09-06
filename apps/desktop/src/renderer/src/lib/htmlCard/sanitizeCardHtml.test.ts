/**
 * The sanitizer is the declarative and navigation gate. It works with the
 * sandbox and CSP — see cardShell.test.ts and tests/e2e/htmlCard.test.ts.
 * What it must guarantee is that nothing a model writes can carry script,
 * events, navigation, an embedded document or an outbound resource load.
 */
import { describe, it, expect } from 'vitest';
import { isSafeStyleAttr, sanitizeCardCss, sanitizeCardHtml } from './sanitizeCardHtml';

const clean = (html: string) => sanitizeCardHtml(html).html;

describe('sanitizeCardHtml', () => {
  it('keeps the markup a card is actually made of', () => {
    const html =
      '<div class="row" id="findings"><h3>Findings</h3><table><thead><tr>' +
      '<th data-wks-sort="number" scope="col">Lines</th></tr></thead>' +
      '<tbody><tr data-wks-filter-item><td colspan="2">412</td></tr></tbody></table>' +
      '<details><summary>Why</summary><p>Because.</p></details>' +
      '<input type="search" data-wks-filter="findings" placeholder="Filter…"></div>';
    const out = clean(html);
    expect(out).toContain('data-wks-sort="number"');
    expect(out).toContain('data-wks-filter-item');
    expect(out).toContain('<details>');
    expect(out).toContain('type="search"');
    expect(out).toContain('colspan="2"');
  });

  it('drops script, style, iframe, form, svg, object and template WITH their contents', () => {
    for (const [markup, leak] of [
      ['<script>alert(1)</script>', 'alert(1)'],
      ['<style>body{color:red}</style>', 'color:red'],
      ['<iframe src="https://evil"></iframe>', 'evil'],
      ['<form action="https://evil"><input name="a"></form>', 'evil'],
      ['<svg><script>alert(1)</script></svg>', 'alert(1)'],
      ['<object data="x.swf"></object>', 'x.swf'],
      ['<template><img src="https://evil/x"></template>', 'evil'],
      ['<meta http-equiv="refresh" content="0;url=https://evil">', 'evil'],
      ['<link rel="stylesheet" href="https://evil/x.css">', 'evil'],
      ['<base href="https://evil/">', 'evil'],
      ['<textarea><img src=x onerror=alert(1)></textarea>', 'onerror'],
    ] as const) {
      const out = clean(markup);
      expect(out, markup).not.toContain(leak);
    }
  });

  it('strips every on* attribute, including ones nobody enumerated', () => {
    const out = clean(
      '<div onclick="a()" onmouseover="b()" ONERROR="c()" onfuturething="d()">hi</div>',
    );
    expect(out).toBe('<div>hi</div>');
  });

  it('removes comments — the other half of every mXSS gadget', () => {
    expect(clean('<p>a</p><!--[if IE]><script>x</script><![endif]--><p>b</p>')).toBe(
      '<p>a</p><p>b</p>',
    );
  });

  it('removes even fragment anchors', () => {
    expect(clean('<a href="#detail">jump</a>')).not.toContain('href=');
    for (const href of [
      'https://evil.test/x',
      'http://127.0.0.1:7891/sessions',
      'javascript:alert(1)',
      'JaVaScRiPt:alert(1)',
      'data:text/html,<script>x</script>',
      'file:///etc/passwd',
      '//evil.test',
      '# with space',
    ]) {
      const out = clean(`<a href="${href}">x</a>`);
      expect(out, href).not.toContain('href=');
    }
  });

  it('never keeps a target or a rel a card chose', () => {
    const out = clean('<a href="#a" target="_blank" rel="opener">x</a>');
    expect(out).not.toContain('_blank');
    expect(out).not.toContain('rel=');
  });

  it('removes all image URLs', () => {
    const ok = '<img src="data:image/png;base64,iVBORw0KGgo=" alt="chart" width="10" height="10">';
    expect(clean(ok)).not.toContain('src=');
    for (const src of [
      'https://evil.test/beacon.png',
      'http://127.0.0.1:7897/x.png',
      'data:image/svg+xml;base64,PHN2Zz48c2NyaXB0Pg==',
      'data:text/html;base64,PHNjcmlwdD4=',
      '/etc/passwd',
    ]) {
      expect(clean(`<img src="${src}">`), src).not.toContain('src=');
    }
  });

  it('refuses a style attribute that could fetch or break out', () => {
    expect(isSafeStyleAttr('color: red; font-weight: 600')).toBe(true);
    for (const value of [
      'background: url(https://evil.test/beacon.png)',
      'background: image-set("https://evil.test/x" 1x)',
      'width: expression(alert(1))',
      'color: red} body { background: url(https://evil.test/x)',
      'color: \\75 rl(https://evil.test/x)',
    ]) {
      expect(isSafeStyleAttr(value), value).toBe(false);
      expect(clean(`<div style="${value.replace(/"/g, '&quot;')}">x</div>`)).not.toContain(
        'style=',
      );
    }
  });

  it('drops an input type that could submit, upload or harvest', () => {
    for (const type of ['file', 'password', 'submit', 'image', 'button', 'hidden']) {
      expect(clean(`<input type="${type}">`), type).toBe('<input type="text">');
    }
    expect(clean('<button onclick="x()">go</button>')).toBe('<button>go</button>');
  });

  it('unwraps an unknown element rather than losing its words', () => {
    expect(clean('<marquee><b>keep me</b></marquee>')).toBe('<b>keep me</b>');
  });

  it('reports what it removed, so the chrome can say so', () => {
    const { removed } = sanitizeCardHtml('<script>x</script><div onclick="y()">z</div>');
    expect(removed.length).toBeGreaterThan(0);
    expect(removed).toContain('onclick');
  });

  it('survives markup shaped to break a naive serializer', () => {
    for (const nasty of [
      '<div>unclosed',
      '</div></body></html><script>x</script>',
      '<p><p><p>'.repeat(50),
      '<table><tr><td>a', // implied tbody
      '<noscript><p title="</noscript><img src=x onerror=alert(1)>">',
    ]) {
      const out = clean(nasty);
      expect(() => out, nasty).not.toThrow();
      expect(out, nasty).not.toContain('onerror');
      expect(out, nasty).not.toContain('<script');
    }
  });
});

describe('sanitizeCardCss', () => {
  it('cannot escape the <style> element the host wraps it in', () => {
    const out = sanitizeCardCss('a{}</style><script>alert(1)</script><style>');
    expect(out).not.toContain('</style>');
    expect(out).not.toContain('<script>');
  });

  it('leaves resource CSS to the independent CSP boundary', () => {
    const out = sanitizeCardCss(
      "@import url('https://evil.test/x.css'); body{background:url(https://evil.test/b.png)}",
    );
    expect(out).toContain('@import');
    expect(out).toContain('evil.test');
  });

  it('leaves ordinary declarations alone', () => {
    const css = '.row{color:var(--wks-text-primary);display:flex;gap:6px}';
    expect(sanitizeCardCss(css)).toBe(css);
  });
});
