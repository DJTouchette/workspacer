package bus

import (
	"fmt"
	"strconv"
)

// AuthorizeLaunchPreparation binds a provider callback to the exact outstanding
// owner spawn it is answering. A node token on its own grants no integration
// access; call ids cannot be borrowed across connections or plugin selections.
func (s *Server) AuthorizeLaunchPreparation(caller CallerIdentity, callID, pluginID string) error {
	id, err := strconv.ParseUint(callID, 10, 64)
	if err != nil {
		return fmt.Errorf("invalid spawn callback identity")
	}
	s.router.mu.Lock()
	defer s.router.mu.Unlock()
	pending := s.router.pending[id]
	provider := s.router.conns[caller.ConnID]
	if pending == nil || pending.method != "agents.spawn" || pending.providerID != caller.ConnID || provider == nil || provider.revoked.Load() || pending.launchIntegrationID == "" || pending.launchIntegrationID != pluginID || !pending.caller.authenticatedHost || pending.caller.revoked.Load() || pending.caller.federated {
		return fmt.Errorf("launch preparation requires this provider's active owner-authorized spawn")
	}
	return nil
}
