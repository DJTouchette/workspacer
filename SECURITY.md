# Security

## Reporting a vulnerability

**Please don't open a public issue for a security problem.**

Report it privately through GitHub:
[open a security advisory](https://github.com/DJTouchette/workspacer/security/advisories/new).
That keeps the report between you and the maintainers until there's a fix.

Useful things to include, if you have them: what an attacker can do, which of
the processes below they need to reach, and the smallest thing that reproduces
it. A concrete path from "attacker position" to "bad outcome" is worth more than
a severity label.

Workspacer is a small project without a paid security team, so there is no
guaranteed response window — but reports get read, and a real one gets a fix.

## What to attack

Workspacer runs a local control plane, and everything below is **loopback-only
by default**:

- `services/hub` (Go) — the event bus and capability broker, plus `brain` (the
  default capability provider), `mcp` (an MCP facade over the bus), and the
  `workspacer` CLI.
- `services/claudemon` (Rust) — the agent session daemon; REST + SSE.
- `apps/desktop` (Electron) — the app itself; the main process is a bus
  capability provider, and the renderer displays untrusted agent output.

Positions worth reasoning from:

- **A web page the user visits** while the daemons run — cross-origin requests
  and DNS rebinding against loopback ports.
- **A remote-share client** — the opt-in `WORKSPACER_REMOTE_SHARE` path, usually
  over Tailscale, where the bus is reachable off-host with a token.
- **An installed plugin** — sandboxed on the filesystem, but it holds a bus
  token and a capability grant.
- **A prompt-injected agent** — model output is untrusted, and an agent reaches
  the bus through the MCP facade.
- **Another local process or user** on a shared machine.

Installing a plugin is a **trusted-install** decision, like a VS Code extension:
a plugin ships code that runs on your machine. What is in scope is a plugin
exceeding the grants its manifest declares, or the install flow running a build
command without asking you first.

## Audit history

This file used to double as the register for a code audit — sixteen numbered
findings with severities, file:line evidence, and the decision taken on each.
Every one is now resolved or has landed as a deliberate, documented decision, so
the register has been retired rather than kept as a list of nothing.

Comments throughout the codebase still cite those items (`SECURITY.md #8` and
so on). They resolve against the last revision that carried the register:

```
git show 4fd1c8f:SECURITY.md
```

Findings from here on are handled as advisories and fixed in the normal commit
history, where the reasoning belongs next to the code anyway.
