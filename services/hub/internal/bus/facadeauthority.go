package bus

// mayAssertLocalSession trusts only the local control plane or an explicitly
// provisioned operator facade. A flag alone on a provider/plugin/peer is inert.
func (cn *conn) mayAssertLocalSession() bool {
	if cn == nil || cn.revoked.Load() || !cn.trusted || cn.pluginID != "" || cn.federated {
		return false
	}
	return !cn.viaScopedToken || (cn.facadeAuthority && (cn.scope == "" || cn.scope == "operator"))
}
