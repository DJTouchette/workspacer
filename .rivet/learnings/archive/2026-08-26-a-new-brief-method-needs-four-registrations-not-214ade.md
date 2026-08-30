---
title: A new brief.* method needs FOUR registrations, not one — capspec, composition, the containment CORPUS, and the tier test
date: 2026-08-26
promoted: true
promoted_to: registration-checklists
---

# A new brief.* method needs FOUR registrations, not one — capspec, composition, the containment CORPUS, and the tier test

## Observation
Adding brief.check taught this the hard way. capspec.go's pathVerbPrefixes contains "brief.", so any new brief.* method is LooksPathBearing and fails closed until it is classified in capspec.PathParam. That much is documented. What is not: TestCorpusMethodsMatchCapspec (cmd/brain/fsguard_test.go) additionally requires an entry in contracts/path-containment-cases.json for EVERY capspec.PathParam method — including desktop-only ones, which declare providers: ["main"] (brief.archive is the precedent). Miss it and the failure reads 'nothing asserts that its provider actually calls the guard', which sounds like a brain problem and is not. Plus internal/capspec/composition.go wants a Compositions() record with a pathGuard witness, and cmd/mcp/tiers_test.go's banned lists must name the new tool so a view/triage scout is pinned out of it. The MCP facade side needs nothing beyond registering the tool: tiers DERIVE from authtoken's exact-name allowlists, so an unlisted brief.* method is operator-only by construction. Also: cmd/mcp/help.go's topic strings are Go RAW string literals, so a backtick anywhere in new help prose is a syntax error.
