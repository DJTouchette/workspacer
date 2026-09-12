package capspec

func init() {
	unscopedByDecision["files.receiveUpload"] = "Hub-owned forwarding endpoint for the existing files.upload permission: spills size-capped image/pdf bytes under the worker UID in a generated per-user temporary directory, 0600 and exclusive creation. No caller path or executable extension is accepted."
	unscopedParams["files.receiveUpload"] = unscopedParams["files.upload"]
	compositionInert["files.receiveUpload"] = InertClaim{Reason: "authenticatedUploadReceiver permits only the hub owner's forwarding call. The same fixed extension/size policy as files.upload applies; bytes become active only if explicitly referenced in an agent message under that tier's existing approval contract. It changes no grant or filesystem root.", Witnesses: []Witness{guarded(argBearing("authenticatedUploadReceiver", "files.receiveUpload", []string{"services", "hub", "internal", "bus", "bus.go"}))}}

	for _, method := range []string{"federation.peersConfig", "federation.savePeersConfig"} {
		unscopedByDecision[method] = "Server-owner administration of peers.json: read returns token presence only; save validates named credential-free WebSocket URLs, preserves omitted tokens, atomically writes mode 0600 and reloads only changed links"
		compositionInert[method] = InertClaim{Reason: "peerConfigTrusted requires authenticated local owner identity. The saved peer credentials and dispatch selection can authorize later peer calls, but this write is restricted to the same server owner who owns that authority; providers, scoped operators, plugins and peer links cannot acquire it.", Witnesses: []Witness{guarded(argBearing("peerConfigTrusted", method, []string{"services", "hub", "cmd", "hub", "peersconfig.go"}))}}
	}
	inertMethods["remote.sharingInfo"] = "no caller parameters; reports whether the server's configured sharing proxy is enabled and whether this caller can manage it"
	for _, method := range []string{"remote.tailscaleInfo", "remote.tailscaleServe", "remote.setSharing"} {
		unscopedByDecision[method] = "Owner-managed network sharing; accepts only an enabled boolean, with hub port and optional private supervisor socket supplied by the launcher. No caller command, port, path, socket or credentials are accepted"
		compositionInert[method] = InertClaim{Reason: "networkTrusted confines listener changes to the authenticated local owner. These changes can expose an already token-protected server, but cannot grant a scoped worker or peer host credentials or root commands. The private combined-node supervisor exposes only Tailscale status and this node's fixed HTTPS proxy.", Witnesses: []Witness{guarded(argBearing("networkTrusted", method, []string{"services", "hub", "cmd", "hub", "networkadmin.go"}))}}
	}

}
