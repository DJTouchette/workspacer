---
title: Metadata final-answer cleanup must retain the child anchor through stderr drain
date: 2026-09-29
promoted: false
---

# Metadata final-answer cleanup must retain the child anchor through stderr drain

## Observation
Actual macOS9666c371 stress reproduced the original two-second JSON dialogue timeout after fixed reply IDs1 and2; protocol answer was complete, but inherited stderr remained open after initial process-group kill. Owner.kill previously disarmed immediately on signal submission, making final cleanup a no-op. JSON exchange now uses signal_retained and repeats the existing identity-fenced group signal every5ms while concurrently draining bounded stderr, inside the unchanged overall timeout. The original child is not reaped during this loop, every Unix attempt repeats WNOWAIT proof, and cancellation retains armed cleanup. An injected lost-anchor test proves ECHILD prevents a second numeric signal. The original fork-after-answer fixture and deadline remain unchanged; actualMac repeat is still required to validate the repair.
