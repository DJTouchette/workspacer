---
title: "Fly node: `brain --help` prints the node's HUB_TOKEN as a flag default"
date: 2026-08-26
promoted: false
---

# Fly node: `brain --help` prints the node's HUB_TOKEN as a flag default

## Observation

`brain --help` on the Fly node renders:

    -token string
        hub bus auth token (empty = no auth) (default "<the node's real 32-char HUB_TOKEN, plaintext>")

Go's `flag` package prints whatever default the flag was constructed with, and
`cmd/brain` seeds `-token` from `$HUB_TOKEN`. 5e4d1eba (`internal/redact`) closed
the dial-error log line, which was the high-volume path; usage output is a
separate surface and still names the credential verbatim. `-hub` has the same
shape but its value carries no secret.

## Impact

Lower severity than the log leak — reading it needs a shell on the machine, and
that shell already has `$HUB_TOKEN` in its environment — but any `brain --help`
captured into a transcript, an agent's tool output or a CI log publishes the
node's bus credential.

## Recommendation

Seed the flag with a placeholder and read the env var after `flag.Parse` (or
override the usage string with a redacted form) so `--help` never names the
value. Audit any other command that defaults a flag from a credential env var.

## Related paths

- `services/hub/cmd/brain/main.go`
- `services/hub/internal/redact/*`

## Confidence

high — observed directly on machine 1857645df24448, 2026-08-26.
