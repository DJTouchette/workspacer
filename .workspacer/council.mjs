export const meta = {
  name: 'reimagine-agent-ui-council',
  description: 'Design council: reimagine the workspacer agent UI paradigm, judge, synthesize a buildable spec',
  phases: [
    { title: 'Propose', detail: '5 divergent paradigm proposals' },
    { title: 'Judge', detail: 'adversarial panel scores each proposal' },
    { title: 'Synthesize', detail: 'merge winner + best grafts into a concrete spec' },
  ],
}

const CONTEXT = [
  '# workspacer — architecture digest (ground truth; verify by reading files)',
  '',
  'Electron + React thin shell over claudemon (Rust daemon owning Claude Code session lifecycle).',
  'A tool for running MANY long-lived Claude Code agents side by side. The core felt use case',
  '(see next-features.md) is babysitting agents you are NOT actively watching — knowing which need',
  'you, attending to them, then moving on.',
  '',
  '## Core data model (src/renderer/src/types/pane.ts, hooks/useAgentManager.ts)',
  '- AgentWorkspace = ONE long-lived claudemon session (by cwd) + a list of TabConfig.',
  '  Fields: id, name, global?, cwd, model?, sessionId?, tabs, activeTabId. Lives in the daemon',
  '  independent of any UI pane. One special global Overview workspace pinned first.',
  '- TabConfig = id, title, panes (PaneConfig[]), activePaneId, canvas? (x,y,w,h), lastActiveAt?',
  '- PaneConfig = id, type, title, cwd, ...  type one of terminal|browser|claude|notes|review|',
  '  library|analytics|overview|plugin|plugins|settings|agent',
  '- ViewMode = tabs | spatial | stacked  — GLOBAL flag (config.panes.viewMode). It currently',
  '  only rearranges the ACTIVE agent TABS, not agents.',
  '',
  '## Live state from claudemon (ClaudeSessionSnapshot), keyed by sessionId, streamed via',
  '## window.electronAPI.onClaudeSessionUpdate(sessionId, snapshot):',
  '- ambientState: idle|thinking|streaming|waiting_input|waiting_approval',
  '- conversation turns, activeToolCalls, completedToolCalls, fileChanges',
  '- pendingApproval (toolName,toolInput,suggestions), pendingQuestions (question,options)',
  '- subagents[], workflows[] (phases + per-agent live tokens/tools) — rich orchestration telemetry',
  '- usage: contextTokens, contextLimit, costUSD, totalInput/OutputTokens, model',
  '- The Rust classifier ALSO detects Stuck / Error / Done but these are NOT surfaced in the UI today',
  '  (dead code: claudemonItems.ts, ItemDetailOverlay.tsx). Reviving them is fair game.',
  '',
  '## Current UI surfaces',
  '- SideBar (196px left): vertical list of agents, each a status dot, context% bar, cost; header',
  '  shows N need you / N working + jump-to-next-attention; spawn button. EXACTLY ONE agent active.',
  '- NavBar (top): tab strip for the ACTIVE agent + viewMode toggle + per-dir scripts.',
  '- ScrollContainer (body): renders the ACTIVE agent tabs in the current viewMode. tabs=horizontal',
  '  snap strip; spatial=pan/zoom canvas of tab-cards with persisted x,y,w,h; stacked=vertical loop feed.',
  '- App.tsx keeps EVERY agent ScrollContainer mounted (display:none when inactive) so switching',
  '  agents never unmounts/kills a Claude pane.',
  '- ClaudePane: per-agent, has a terminal mode AND a rich GUI mode (conversation, approve/answer',
  '  buttons, diff view, subagent rows, workflow run cards).',
  '- OverviewPane: cross-agent stats (agents/working/needsYou/cost) + recent/favourite dirs to spawn.',
  '- CommandPalette, vim/default keybindings (leader chords), Claude profiles, layouts, remote',
  '  control over a hub bus (phone/PC), a sibling Rust TUI (wks-tui) over the same bus.',
  '',
  '## THE LOAD-BEARING CONSTRAINT (do not break; cite in any proposal)',
  'Switching view modes / rearranging must NOT re-parent the pane DOM nodes. Terminals (xterm),',
  'webviews, and Claude viewers remount and die if their DOM subtree is moved. ScrollContainer keeps',
  'ONE pane-host subtree with stable React keys; only CSS/geometry changes between modes',
  '(display:contents in tabs, absolute+transform in spatial). Any new paradigm must preserve live',
  'pane DOM identity.',
  '',
  '## THE GAP (the reimagining target)',
  'The view modes operate WITHIN one agent. There is no cross-AGENT view: no mission control where',
  'every agent is a live, glanceable card you watch at once, attention flows to you, and you can zoom',
  'into one agent full workspace. The user tunnels into one agent at a time. The rich per-session',
  'telemetry (ambientState, pendingApproval, subagents, workflows, stuck/error) is mostly hidden',
  'behind one-agent-at-a-time.',
].join('\n')

const PROPOSAL_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['name','tagline','philosophy','coreLoop','reusedPrimitives','newConcepts','keyInteractions','whyFitsBabysitting','respectsNoRemount','risks','mvpScope','stretch'],
  properties: {
    name: { type: 'string', description: 'Memorable name for the paradigm' },
    tagline: { type: 'string', description: 'One sentence pitch' },
    philosophy: { type: 'string', description: 'Core design philosophy / mental model (2-4 sentences)' },
    coreLoop: { type: 'string', description: 'The moment-to-moment interaction loop the user lives in' },
    reusedPrimitives: { type: 'array', items: { type: 'string' }, description: 'Existing primitives/components this builds on' },
    newConcepts: { type: 'array', items: { type: 'string' }, description: 'New concepts/components introduced' },
    keyInteractions: { type: 'array', items: { type: 'string' }, description: 'Concrete interactions (keys, clicks, gestures) and what they do' },
    whyFitsBabysitting: { type: 'string', description: 'Why this serves watching many agents you are not actively looking at' },
    respectsNoRemount: { type: 'string', description: 'How it preserves live pane DOM identity' },
    risks: { type: 'array', items: { type: 'string' } },
    mvpScope: { type: 'array', items: { type: 'string' }, description: 'Smallest shippable slice, ordered' },
    stretch: { type: 'array', items: { type: 'string' }, description: 'Ambitious extensions if time allows' },
  },
}

const LENSES = [
  { key: 'mission-control', prompt: 'Lens: MISSION CONTROL / FLEET DECK. Promote AGENTS (not tabs) to first-class cards on a single deck. The home screen is all agents at once, each a live glanceable card (status, current tool, pending approval, last message, context%, subagent/workflow progress). Attention routing is the spine: the deck reorders/highlights by who-needs-you. Clicking an agent zooms into its full workspace (today per-agent tabs view). Think NORAD / air-traffic-control for agents. Push hard on glanceability and the zoom-in/zoom-out gesture.' },
  { key: 'infinite-canvas', prompt: 'Lens: ONE INFINITE SPATIAL CANVAS with SEMANTIC ZOOM. No separate agent and tab levels — everything (every agent, every pane) lives at a position in one boundless zoomable world. Zoomed out you see the whole fleet as tiles; zoom in and a tile becomes the live agent workspace; zoom further into a single pane. Spatial memory (my refactor agent lives top-left) replaces navigation. Push on semantic level-of-detail rendering and how the existing spatial canvas generalizes to agents.' },
  { key: 'attention-feed', prompt: 'Lens: AMBIENT ATTENTION FEED. A single prioritized stream is the primary surface — every event that might want you (approval needed, question, done, stuck, error, big diff) becomes a card in a feed you triage like an inbox, responding INLINE (approve/answer/dismiss) without leaving the feed. The classifier Stuck/Error/Done (currently dead code) is the fuel. Agents recede; their REQUESTS are the unit. Push on inline-resolution and never-having-to-go-find-the-agent.' },
  { key: 'tiling-compositor', prompt: 'Lens: KEYBOARD-DRIVEN TILING COMPOSITOR (vim-native). A dense tiling grid of live agents, driven almost entirely by the keyboard, modal like vim/tmux. Leader chords to focus/swap/zoom/spawn/broadcast. No mouse needed. The unit is a viewport you tile, swap, and stack. Mirrors the sibling wks-tui tiling-tree+buffer model so the two converge. Push on speed, muscle memory, and density.' },
  { key: 'orchestrator', prompt: 'Lens: CONVERSATIONAL ORCHESTRATOR / ORG CHART. You do not manage windows — you talk to a meta-orchestrator (a top-level command bar / conductor) that arranges agents for you. Agents are an org chart / dependency graph: one agent completion can trigger another, you broadcast a prompt to many, you see the team as a graph of who is working on what. The workspace LAYS ITSELF OUT in response to intent. Push on agent-orchestration (the unbuilt Prompts, runs, workflows ambition) and on the workspace as a managed team rather than manual windows.' },
]

phase('Propose')
log('Convening a 5-member design council, each with a distinct philosophy...')
const proposalsRaw = await parallel(LENSES.map((lens) => () =>
  agent(
    CONTEXT + '\n\nYou are a member of a design council reimagining the workspacer agent UI. Argue HARD for your assigned lens — be bold and specific, this is a go-out-there brief. Ground yourself in the real code: you may read src/renderer/src/components/ScrollContainer.tsx, components/SideBar.tsx, hooks/useAgentManager.ts, types/pane.ts, types/claudeSession.ts, panes/ClaudePane.tsx, panes/OverviewPane.tsx to make your proposal concrete and feasible. Reuse existing primitives where sane, respect the no-remount constraint, and serve the babysitting-many-agents use case. Be concrete about components, data-model changes, and the moment-to-moment loop.\n\n' + lens.prompt + '\n\nReturn ONLY the structured proposal.',
    { label: 'propose:' + lens.key, phase: 'Propose', schema: PROPOSAL_SCHEMA },
  ),
))
const proposals = proposalsRaw.filter(Boolean)

phase('Judge')
log(proposals.length + ' proposals in. Convening an adversarial judge panel...')
const JUDGE_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['ranking','scores','bestIdeasAcrossAll','verdict'],
  properties: {
    ranking: { type: 'array', items: { type: 'string' }, description: 'Proposal names, best first' },
    scores: { type: 'array', items: {
      type: 'object', additionalProperties: false,
      required: ['name','fitToUser','ambition','feasibility','valueForEffort','coherence','total','critique'],
      properties: {
        name: { type: 'string' },
        fitToUser: { type: 'number', description: '0-10 serves babysitting-many-agents' },
        ambition: { type: 'number', description: '0-10 boldness' },
        feasibility: { type: 'number', description: '0-10 buildable on existing primitives incl no-remount' },
        valueForEffort: { type: 'number', description: '0-10' },
        coherence: { type: 'number', description: '0-10 internal consistency / clarity' },
        total: { type: 'number' },
        critique: { type: 'string' },
      },
    } },
    bestIdeasAcrossAll: { type: 'array', items: { type: 'string' }, description: 'Strongest individual ideas across all proposals, worth grafting into the winner' },
    verdict: { type: 'string', description: 'Which paradigm should win and why, ambitious yet shippable' },
  },
}
const proposalsBlob = proposals.map((p, i) => 'Proposal ' + (i+1) + ': ' + p.name + '\nTagline: ' + p.tagline + '\nPhilosophy: ' + p.philosophy + '\nCore loop: ' + p.coreLoop + '\nNew concepts: ' + (p.newConcepts||[]).join('; ') + '\nKey interactions: ' + (p.keyInteractions||[]).join('; ') + '\nWhy fits babysitting: ' + p.whyFitsBabysitting + '\nNo-remount: ' + p.respectsNoRemount + '\nMVP: ' + (p.mvpScope||[]).join('; ') + '\nRisks: ' + (p.risks||[]).join('; ')).join('\n\n')

const JUDGE_LENSES = [
  'You are the PRAGMATIST judge. Weight feasibility, the no-remount constraint, and value-for-effort highest. Punish hand-waving.',
  'You are the VISIONARY judge. Weight ambition and how transformative the daily experience is. Punish timidity and incrementalism.',
  'You are the USER-ADVOCATE judge. You ARE the user who babysits many agents. Weight fit-to-actual-workflow highest. Punish anything that adds friction to who-needs-me-handle-it-move-on.',
]
const judgmentsRaw = await parallel(JUDGE_LENSES.map((jl, i) => () =>
  agent(
    CONTEXT + '\n\nYou are judging a design council reimagining the workspacer agent UI.\n\n' + jl + '\n\nScore every proposal on each axis (0-10), rank them, name the best individual ideas across ALL proposals worth grafting, and give a verdict on which should win — it must be both ambitious AND shippable. Proposals:\n\n' + proposalsBlob + '\n\nReturn ONLY the structured judgment.',
    { label: 'judge:' + (i+1), phase: 'Judge', schema: JUDGE_SCHEMA },
  ),
))
const judgments = judgmentsRaw.filter(Boolean)

phase('Synthesize')
log('Judges have voted. Synthesizing the winning paradigm into a concrete, buildable spec...')
const roles = ['pragmatist','visionary','user-advocate']
const judgeBlob = judgments.map((j, i) => 'Judge ' + (i+1) + ' (' + roles[i] + ')\nRanking: ' + (j.ranking||[]).join(' > ') + '\nVerdict: ' + j.verdict + '\nBest ideas to graft: ' + (j.bestIdeasAcrossAll||[]).join('; ') + '\nScores: ' + (j.scores||[]).map((s) => s.name + '=' + s.total).join(', ')).join('\n\n')

const SPEC_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['paradigmName','tagline','elevatorPitch','winningBackbone','graftedIdeas','mentalModel','dataModelChanges','componentPlan','interactions','noRemountStrategy','mvpChecklist','futureWork','whyThisWins'],
  properties: {
    paradigmName: { type: 'string' },
    tagline: { type: 'string' },
    elevatorPitch: { type: 'string', description: '3-5 sentences a user would read' },
    winningBackbone: { type: 'string', description: 'Which proposal is the backbone and why' },
    graftedIdeas: { type: 'array', items: { type: 'string' }, description: 'Ideas grafted from other proposals' },
    mentalModel: { type: 'string', description: 'The new mental model in one paragraph' },
    dataModelChanges: { type: 'array', items: { type: 'string' }, description: 'Concrete type/state changes (ViewMode additions, config flags, new fields)' },
    componentPlan: { type: 'array', items: {
      type: 'object', additionalProperties: false,
      required: ['component','action','detail'],
      properties: {
        component: { type: 'string' },
        action: { type: 'string', enum: ['create','modify','reuse'] },
        detail: { type: 'string' },
      },
    } },
    interactions: { type: 'array', items: { type: 'string' }, description: 'Concrete keybindings/clicks/gestures and their effect' },
    noRemountStrategy: { type: 'string', description: 'Exactly how live pane DOM identity is preserved' },
    mvpChecklist: { type: 'array', items: { type: 'string' }, description: 'Ordered, concrete build steps for a shippable MVP' },
    futureWork: { type: 'array', items: { type: 'string' } },
    whyThisWins: { type: 'string' },
  },
}
const spec = await agent(
  CONTEXT + '\n\nYou are the council synthesizer. The proposals have been judged. Produce ONE concrete, buildable spec for the reimagined paradigm. Choose the strongest BACKBONE proposal, then GRAFT the best ideas from the others (the judges named several). Ambitious but shippable in this codebase, respecting the no-remount constraint exactly, serving babysitting-many-agents. Be concrete: name real files/components to create or modify, real type changes, real keybindings. The MVP checklist must be implementable step-by-step.\n\n## The proposals\n' + proposalsBlob + '\n\n## The judges\n' + judgeBlob + '\n\nReturn ONLY the structured spec.',
  { label: 'synthesize', phase: 'Synthesize', schema: SPEC_SCHEMA },
)

return { proposals, judgments, spec }
