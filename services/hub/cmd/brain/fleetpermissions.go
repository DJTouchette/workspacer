package main

// Fleet full access is a provider approval preference, never a token grant.
// Consult current config for every launch and follow recorded parent lineage.
func (r *registry) fleetSkipsPermissions(p spawnParams) bool {
	agents, _ := r.cfg.get()["agents"].(map[string]any)
	enabled, _ := agents["fleetFullAccess"].(bool)
	if !enabled {
		return false
	}
	if p.Manager {
		return true
	}
	seen := map[string]bool{}
	for id := p.ParentSessionID; id != "" && !seen[id] && r.meta != nil; {
		seen[id] = true
		parent, ok := r.meta.get(id)
		if !ok {
			return false
		}
		if parent.IsWakeTarget {
			return true
		}
		id = parent.ParentSessionID
	}
	return false
}
