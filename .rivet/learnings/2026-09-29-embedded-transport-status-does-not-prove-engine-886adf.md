---
title: Embedded transport status does not prove engine rejection
date: 2026-09-29
promoted: false
---

# Embedded transport status does not prove engine rejection

## Observation
Manager request audit found EmbeddedClient converted every non-2xx response into CommandRejected, causing lifecycle/outbox code to release reservations after ambiguous500 responses. Preserve the existing deliverCaptured boundary: only400/401/403/404/409/410/413/422/429 and503 with exact body session input queue is full are definitive. Other statuses are unknown outcomes even if HTTP completed, because execution may precede response failure. Regression simulates an accepted effect followed by500 and refuses the definitive-rejection marker.
