---
title: Monitoring a Tokio child must retain the parentwatch stdin lease
date: 2026-09-27
promoted: false
---

# Monitoring a Tokio child must retain the parentwatch stdin lease

## Observation
Tokio Child::wait closes Child.stdin on its first poll. Native LocalServices used wait in a liveness select, so the real embedded smoke caused workspacer parentwatch to receive EOF and shut down healthy services immediately after readiness. OwnedGateway now takes ChildStdin out at spawn and holds it separately until intentional shutdown. The regression polls wait with stdin retained, cancels that wait, then closes the lease and successfully waits again.
