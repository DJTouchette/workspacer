---
title: Node bus tokens are only distinguishable as SCOPED tokens — the hub has no provider/node identity at the HTTP layer
date: 2026-08-25
confidence: high
suggested_doc: hub-bus-control-plane
related_paths:
  - services/hub/cmd/hub/hostonly.go
  - services/hub/internal/bus/bus.go
  - services/hub/internal/capspec/httproutes.go
  - deploy/fly/node/RUNBOOK.md
promoted: true
promoted_to: hub-bus-control-plane
---

# Node bus tokens are only distinguishable as SCOPED tokens — the hub has no provider/node identity at the HTTP layer

## Observation
A remote worker node (deploy/fly/node) authenticates with an operator-tier scoped token from tokens.json. At the hub's HTTP layer there is NO way to tell "a node" from "the user's own operator pairing": bus.Server.Authorized admits the host token and any operator-scoped token identically, ScopedIdent carries only Scope/Methods/grants (now also Label), and the provider-ness of a connection (conn.mayProvide) is a property of a live /bus WebSocket, not of an HTTP request — so gating on "is currently a provider" would be trivially evaded by a bare POST carrying the token. The only real, non-evadable distinction available today is host token vs. scoped token: the host token is a file on the hub (<config>/workspacer/remote-token) that no node is ever handed, and every local caller (desktop hubAuthHeaders→getHubToken, the CLI, `workspacer plugin dev`) presents it, while nodes are minted `workspacer token create --scope operator`.</observation>
<parameter name="impact">This is why /plugins/install, /plugins/examples/install and /plugins/reload are now wrapped in cmd/hub/hostOnlyRoute (bus.Server.HostAuthorized) rather than guard(): those three run code on the hub host and guard() admitted a node's token. It also bounds the gate — a node handed the HOST token is indistinguishable from the host, and no credential-keyed check can see that.

## Recommendation
Any future "refuse this to nodes" gate must key on the credential (scoped vs host), not on the connection: HTTP routes have no bus connection to inspect. The durable fix is a provider tier so conn.mayProvide stops promoting a connection to trusted; until then, capspec's RouteHostOnly disposition marks which routes need the stronger gate and its completeness guard pins the wrapper in both directions.
