---
title: The brief line's DOUBLE SPACE is load-bearing — never \s+ collapse a manager's sentence
date: 2026-08-26
promoted: true
promoted_to: fleet-manager
---

# The brief line's DOUBLE SPACE is load-bearing — never \s+ collapse a manager's sentence

## Observation
briefService.normalizeBriefLine already warns about this and it is easy to reintroduce one layer up: the doctrine's dated-log format is '- YYYY-MM-DD  <what happened>' with TWO spaces, so a \s+ -> ' ' flatten in any new brief-line composer silently re-spaces the one format the brief tooling exists to write. Caught by a test on composeResultLine's 'the caller already dated their sentence' path. The correct flatten is normalizeBriefLine's two replaces — newline runs and tab/FF/VT runs become one space, interior SPACES are left alone — and the Go twin's flattenBriefLine (cmd/brain/brief.go) is the same function, so both providers must use it rather than strings.Fields or a \s+ regexp.
