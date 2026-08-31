package main

// WIRING THE CEILING TO THE SPAWN GATE — the two functions that make a routing
// decision binding rather than advisory.
//
// The seam runs exactly here. internal/bus owns the CLAMP: it is the one
// spawn-path function in this repo that is not a twin, and because
// methodSanitizers is the single dispatch table for both call() and
// federatedCall(), a clamp there covers the federated hop by construction.
// internal/routing owns the POLICY: the matrix, the capability ladder, the
// per-directory ceilings, and the reasoning that produces a verdict. Neither
// imports the other. This file is the wire between them, and it is deliberately
// thin — every judgement below is made by routing.Matrix.CheckSpawn; nothing
// here decides anything.
//
// TWO JUDGEMENTS TRAVEL THIS WIRE, not one. The ceiling clamps how much
// capability and authority a directory allows. The FRESHNESS arm refuses a
// spawn that declared review work and also asked to resume the session it is
// meant to be reviewing. They share this resolver because they share the
// enforcement site, and the enforcement site is the single spawn-path function
// that is not a twin; splitting them into two injected hooks would mean two
// resolvers reading the same matrix on the same call.
//
// THE CANONICALIZATION IS THE ROUTER'S, NOT OURS, and that is not an accident of
// layering. CeilingFor is a LEXICAL ancestor match: hand it a caller's spelling
// and a symlink walks straight around the ceiling. The router resolves the cwd
// with the same walk its filesystem guard uses before it calls out, so there is
// ONE canonicalizer on this path rather than two that can disagree — the same
// check-path/opened-path rule, applied to a directory that selects a policy row
// instead of one that opens a file.

import (
	"log"

	"github.com/djtouchette/workspacer-hub/internal/bus"
	"github.com/djtouchette/workspacer-hub/internal/routing"
)

// routingSpawnCeiling is the resolver the router calls for every bus
// agents.spawn.
//
// It reads the matrix IN FORCE at this instant rather than one captured at
// startup, so an edit to routing.yaml's `ceilings:` block binds on the next tick
// — the same live-reload contract routing.select answers under. A hub running on
// the compiled-in defaults (no file, or a file that does not parse) still has a
// `default` ceiling, because the merge cannot delete one.
//
// AN UNRESOLVABLE cwd GETS THE DEFAULT CEILING, never no ceiling. A directory
// that does not exist yet, a relative path, a spelling the canonicalizer refuses
// — all of them arrive here with CwdResolved false and an empty CanonicalCwd,
// which CeilingFor answers with the `default` entry. The alternative, treating
// "we could not resolve it" as "unconstrained", would make the ceiling optional
// for anyone willing to spell the path badly.
func routingSpawnCeiling(svc *routing.Service) bus.SpawnCeilingFunc {
	return func(req bus.SpawnCeilingRequest) bus.SpawnCeilingVerdict {
		m := svc.Matrix()
		if m == nil {
			return bus.SpawnCeilingVerdict{}
		}
		if !req.CwdResolved && req.CanonicalCwd == "" {
			// Said once per spawn rather than kept quiet: a ceiling applied to the
			// wrong row is worth knowing about, and "the default applied because we
			// could not resolve your cwd" is a different fact from "the default
			// applied because you have no entry".
			log.Printf("[routing] agents.spawn: the spawn's cwd could not be canonicalized, so the %q ceiling applies rather than any per-directory one", routing.CeilingDefaultKey)
		}
		v := m.CheckSpawn(routing.SpawnRequest{
			CanonicalCwd:    req.CanonicalCwd,
			Capability:      req.Capability,
			Role:            req.Role,
			Resuming:        req.Resuming,
			ResumeSessionID: req.ResumeSessionID,
			ToolScope:       req.ToolScope,
			Provider:        req.Provider,
			Model:           req.Model,
			Effort:          req.Effort,
		})
		return bus.SpawnCeilingVerdict{
			Key:               v.Key,
			MaxCapability:     v.MaxCapability,
			MaxToolScope:      v.MaxToolScope,
			CapabilityRefused: v.CapabilityRefused,
			Capability:        v.Capability,
			ToolScopeRefused:  v.ToolScopeRefused,
			ToolScope:         v.ToolScope,
			Provider:          v.Provider,
			Model:             v.Model,
			Effort:            v.Effort,
			ResumeRefused:     v.ResumeRefused,
			FreshCapability:   v.FreshCapability,
			Denied:            v.Denied,
			Because:           v.Because,
		}
	}
}

// routingSpawnAudit writes the SPAWN half of the decision log.
//
// This is the row that makes `decisionId` mean something. routing.select writes
// the decision; this writes the spawn that quoted it; the id joins them. Without
// this half the id on the wire would be a field nothing ever reads, which is
// this repo's most common bug and one it has shipped before.
//
// It is also the only place a headless node records what its workers were: the
// desktop's analytics store (workspacer.db, session_history) is written by the
// Electron main process and a hub-only node has none, so on a multi-node fleet
// this file is the whole record.
//
// It must not block — it runs in front of a caller waiting for its spawn — and
// it does not: one O_APPEND write of one line, with every failure swallowed into
// a log line.
func routingSpawnAudit(logf *routing.DecisionLog) bus.SpawnAuditFunc {
	return func(r bus.SpawnRecord) {
		ceiling := routing.CeilingVerdict{
			Key:               r.Ceiling.Key,
			MaxCapability:     r.Ceiling.MaxCapability,
			MaxToolScope:      r.Ceiling.MaxToolScope,
			CapabilityRefused: r.Ceiling.CapabilityRefused,
			Capability:        r.Ceiling.Capability,
			ToolScopeRefused:  r.Ceiling.ToolScopeRefused,
			ToolScope:         r.Ceiling.ToolScope,
			Provider:          r.Ceiling.Provider,
			Model:             r.Ceiling.Model,
			Effort:            r.Ceiling.Effort,
			ResumeRefused:     r.Ceiling.ResumeRefused,
			FreshCapability:   r.Ceiling.FreshCapability,
			Denied:            r.Ceiling.Denied,
			Because:           r.Ceiling.Because,
		}
		logf.Spawn(r.DecisionID, routing.SpawnEntry{
			Role:          r.Role,
			Capability:    r.Capability,
			Cwd:           r.Cwd,
			Provider:      r.Provider,
			Model:         r.Model,
			Effort:        r.Effort,
			ToolScope:     r.ToolScope,
			CallerScope:   r.CallerScope,
			CallerTokenID: r.CallerTokenID,
			Ceiling:       &ceiling,
			Scrubbed:      r.Scrubbed,
		})
	}
}
