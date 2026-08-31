package plugin

import (
	"encoding/json"
	"log"
	"os"
	"path/filepath"
	"slices"
	"strings"
)

// THE GRANT PIN — the consented authority of a plugin, recorded OUTSIDE the one
// directory that plugin can write.
//
// The sandbox and the loader were each correct in isolation and self-referential
// together:
//
//	sandbox.Policy{WriteRoots: []string{mf.Dir}}      manager.go — the sidecar's
//	                                                  ONLY writable location on
//	                                                  the host is its own dir
//	l.token = loadOrCreatePluginToken(mf.Dir)         …and that dir holds
//	m.reg.RegisterPluginToken(l.token, mf.ID,         plugin.json, which is the
//	    grantsFor(mf), eventGrantsFor(mf))            sole input to the grant
//
// grantsFor is a pure function of the CURRENT on-disk manifest, and
// loadOrCreatePluginToken ADOPTS the existing .bus-token. So a sidecar installed
// with the narrowest possible grant (fs.read scoped to ${pluginDir}) could
// overwrite its own plugin.json — the one file inside its one writable root —
// with `capabilities: [fs.read "/", fs.write "/", agents.spawn, terminals.create,
// config.save, sessions.save]`, and on the next hub start the SAME persisted
// token was registered with all of it. No reinstall, no tarball, no version
// bump, no consent prompt. Verified: the grant list printed before and after the
// rewrite, with the token unchanged.
//
// The crossing is WRITE-THEN-INTERPRET across the boundary built to contain it:
// bytes the sandbox permits as plugin DATA are re-read as the plugin's
// CAPABILITY GRANT. install.go already holds the right principle for the
// neighbouring field — "consent is to a specific command, not to the idea of
// one", re-checked against the argv after re-download — and `capabilities` had
// no analogue. Note too that .bus-token, .settings.json, .disabled and
// .install-source are defended against a hostile TARBALL by
// stripLoaderOwnedSidecars ("an archive or source tree supplying one is always
// either a mistake or an attack") while plugin.json, the file that decides the
// grant, was not treated as loader-owned against the RUNNING sidecar.
//
// The fix is not an OS one. Roughly half the sidecars announce
// `plugin.unsandboxed`, and the confinement mechanism differs per platform, so a
// bind-mount trick would hold on one OS and nowhere else. Instead the LOADER
// keeps the consented surface where the plugin cannot reach it and refuses
// anything the manifest later adds. An operator action that RE-obtains consent —
// install, update, or an explicit /plugins/reload — re-baselines the pin, which
// is what makes a legitimate plugin update work; an automatic load (boot,
// settings change, enable/disable) may only narrow.

// grantPinDirName is a dot-directory so LoadDir skips it: dot-prefixed entries
// under the plugins dir are loader work areas, never plugins.
const grantPinDirName = ".grants"

// grantPin is the declared authority surface a plugin was consented to. It pins
// the MANIFEST's declarations rather than the resolved roots, because the same
// declaration resolves differently per token (${pluginDir} at load,
// ${agentCwd} per pane) and it is the declaration a human approved.
type grantPin struct {
	Capabilities []Capability `json:"capabilities"`
	Emits        []string     `json:"emits,omitempty"`
	Consumes     []string     `json:"consumes,omitempty"`
	Provides     []string     `json:"provides,omitempty"`
}

func grantPinPath(mf Manifest) string {
	if mf.Dir == "" {
		return ""
	}
	// Key the pin on the DIRECTORY the plugin lives in, not mf.ID. mf.ID is read
	// from plugin.json, which sits inside the sandbox's one write root, so a
	// sidecar can rename its own id — and a pin keyed on the id would then point
	// at a file that does not exist, which reads as "never seen before" and
	// trust-on-first-load re-baselines the (now escalated) manifest. The
	// directory basename is the identity install created (dir = sanitizeName(id))
	// and the sidecar cannot change it: renaming a directory needs write access
	// to the PARENT, which the sandbox does not grant. So it is the one stable
	// name the plugin cannot forge.
	base := filepath.Base(mf.Dir)
	if base == "" || base == "." || base == string(filepath.Separator) {
		return ""
	}
	return filepath.Join(filepath.Dir(mf.Dir), grantPinDirName, base+".json")
}

// RebaselineGrantPin records the manifest's declared authority as consented.
// Called only from paths where a human re-approved this manifest: install,
// update, and the explicit operator-guarded reload. Everything else may narrow
// the pin but never widen it.
func RebaselineGrantPin(mf Manifest) {
	path := grantPinPath(mf)
	if path == "" {
		return
	}
	writeGrantPin(path, pinOf(mf))
}

func pinOf(mf Manifest) grantPin {
	return grantPin{
		Capabilities: mf.Capabilities,
		Emits:        mf.Emits,
		Consumes:     mf.Consumes,
		Provides:     mf.Provides,
	}
}

func writeGrantPin(path string, p grantPin) bool {
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		log.Printf("[plugin] ERROR: cannot create the grant-pin directory %s: %v", filepath.Dir(path), err)
		return false
	}
	data, err := json.MarshalIndent(p, "", "  ")
	if err != nil {
		return false
	}
	if err := os.WriteFile(path, data, 0o600); err != nil {
		log.Printf("[plugin] ERROR: cannot write the grant pin %s: %v", path, err)
		return false
	}
	return true
}

// ensureGrantPin records the pin the FIRST time the hub loads a plugin that has
// none — trust on first load, which is the same answer the install flow gives
// (the consent dialog showed exactly this manifest) and the only one that does
// not revoke every already-installed plugin the moment this code ships.
//
// Called from Manager.Add rather than from the grant derivation, deliberately:
// deriving a grant must have no side effects, or every test and every read-only
// inspection of a manifest would leave a file behind next to it.
func ensureGrantPin(mf Manifest) {
	path := grantPinPath(mf)
	if path == "" {
		return
	}
	if _, err := os.Stat(path); err == nil {
		return
	} else if !os.IsNotExist(err) {
		return // unreadable: consentedManifest fails closed on it
	}
	if !writeGrantPin(path, pinOf(mf)) {
		// No pin on disk and none can be written. The manifest's own
		// declarations still apply — refusing every grant would break a
		// legitimately read-only install — but say so, loudly, because this is
		// the state the escalation lives in.
		log.Printf("SECURITY: plugin %s: no grant pin could be recorded at %s, so its capabilities are whatever plugin.json says on each load. A sidecar can write its own plugin.json (it is inside the sandbox's only write root), so this configuration cannot detect self-escalation.", mf.ID, path)
	}
}

// consentedManifest returns mf with its declared authority narrowed to the pin.
// Read-only: an absent pin means "not yet recorded" and the manifest stands (see
// ensureGrantPin).
//
// A pin that exists but cannot be READ fails CLOSED: an unreadable or corrupt
// pin is the tamper case, and "the record of what you were allowed is damaged"
// must not resolve to "take what the file says".
func consentedManifest(mf Manifest) Manifest {
	path := grantPinPath(mf)
	if path == "" {
		return mf
	}
	data, err := os.ReadFile(path)
	if os.IsNotExist(err) {
		return mf
	}
	if err != nil {
		log.Printf("SECURITY: plugin %s: grant pin %s is unreadable (%v) — granting nothing rather than trusting plugin.json, which the plugin itself can write.", mf.ID, path, err)
		return withNoAuthority(mf)
	}
	var pin grantPin
	if json.Unmarshal(data, &pin) != nil {
		log.Printf("SECURITY: plugin %s: grant pin %s is corrupt — granting nothing rather than trusting plugin.json, which the plugin itself can write.", mf.ID, path)
		return withNoAuthority(mf)
	}
	return narrowToPin(mf, pin)
}

func withNoAuthority(mf Manifest) Manifest {
	mf.Capabilities = nil
	mf.Emits, mf.Consumes, mf.Provides = nil, nil, nil
	return mf
}

// narrowToPin drops every declaration the pin does not contain. A capability
// keeps only the path scopes that were consented; a capability whose METHOD is
// new is dropped whole.
func narrowToPin(mf Manifest, pin grantPin) Manifest {
	pinned := map[string][]string{}
	// The CHILD-DELEGATION grant is pinned separately from the paths, and it has
	// to be: `{"method":"agents.spawn","childToolScope":"operator"}` added to
	// plugin.json after install would otherwise ride in on a consent the user
	// gave to a bare agents.spawn — the exact write-then-interpret crossing this
	// whole file exists to close, on the one field that mints child authority.
	pinnedChildScope := map[string]string{}
	for _, c := range pin.Capabilities {
		pinned[c.Method] = append(pinned[c.Method], c.Paths...)
		if c.ChildToolScope != "" {
			pinnedChildScope[c.Method] = c.ChildToolScope
		}
	}
	caps := make([]Capability, 0, len(mf.Capabilities))
	for _, c := range mf.Capabilities {
		paths, ok := pinned[c.Method]
		if !ok {
			log.Printf("SECURITY: plugin %s: dropping capability %q — it is not in the grant pin, so it was added to plugin.json AFTER the user consented to this plugin. Reinstall (or explicitly reload) to re-obtain consent.", mf.ID, c.Method)
			continue
		}
		if c.ChildToolScope != "" && !strings.EqualFold(c.ChildToolScope, pinnedChildScope[c.Method]) {
			log.Printf("SECURITY: plugin %s: dropping childToolScope %q from capability %q — the grant pin records %q, so this child-delegation grant was added to (or widened in) plugin.json AFTER the user consented. The plugin may still spawn; its workers get no workspacer tools. Reinstall (or explicitly reload) to re-obtain consent.",
				mf.ID, c.ChildToolScope, c.Method, pinnedChildScope[c.Method])
			c.ChildToolScope = pinnedChildScope[c.Method]
		}
		kept := make([]string, 0, len(c.Paths))
		for _, p := range c.Paths {
			if slices.Contains(paths, p) {
				kept = append(kept, p)
				continue
			}
			log.Printf("SECURITY: plugin %s: dropping path scope %q from capability %q — it is not in the grant pin (consented: %v).", mf.ID, p, c.Method, paths)
		}
		if len(c.Paths) > 0 && len(kept) == 0 {
			// Every declared scope was new. A path-scoped capability with no
			// roots is granted nothing by the bus anyway; dropping it whole
			// keeps the log honest about what happened.
			continue
		}
		c.Paths = kept
		caps = append(caps, c)
	}
	mf.Capabilities = caps
	mf.Emits = narrowPatterns(mf.ID, "emits", mf.Emits, pin.Emits)
	mf.Consumes = narrowPatterns(mf.ID, "consumes", mf.Consumes, pin.Consumes)
	mf.Provides = narrowPatterns(mf.ID, "provides", mf.Provides, pin.Provides)
	return mf
}

func narrowPatterns(id, field string, declared, pinned []string) []string {
	if len(declared) == 0 {
		return declared
	}
	out := make([]string, 0, len(declared))
	for _, p := range declared {
		if slices.Contains(pinned, p) {
			out = append(out, p)
			continue
		}
		log.Printf("SECURITY: plugin %s: dropping %s pattern %q — it is not in the grant pin, so it was added after consent.", id, field, p)
	}
	return out
}
