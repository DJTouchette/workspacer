package plugin

import (
	"strconv"
	"strings"
	"sync"
)

// UpdateStatus is the result of checking one installed plugin against its
// install source: whether a newer published version exists.
type UpdateStatus struct {
	ID        string `json:"id"`
	Installed string `json:"installed,omitempty"` // version currently on disk
	Latest    string `json:"latest,omitempty"`    // version at the install source
	HasUpdate bool   `json:"hasUpdate"`           // Latest is a strict upgrade over Installed
	Error     string `json:"error,omitempty"`     // source unreachable / unreadable
}

// CompareVersions compares two dotted version strings (an optional leading "v"
// and any "-prerelease"/"+build" suffix are ignored), returning -1 if a < b,
// 0 if equal, and 1 if a > b. Segments are compared numerically when both are
// numbers, else lexically; a missing segment counts as 0 so "1.2" == "1.2.0".
func CompareVersions(a, b string) int {
	as, bs := splitVersion(a), splitVersion(b)
	n := len(as)
	if len(bs) > n {
		n = len(bs)
	}
	for i := 0; i < n; i++ {
		av, bv := "0", "0"
		if i < len(as) && as[i] != "" {
			av = as[i]
		}
		if i < len(bs) && bs[i] != "" {
			bv = bs[i]
		}
		ai, aerr := strconv.Atoi(av)
		bi, berr := strconv.Atoi(bv)
		if aerr == nil && berr == nil {
			if ai != bi {
				if ai < bi {
					return -1
				}
				return 1
			}
			continue
		}
		if av != bv {
			if av < bv {
				return -1
			}
			return 1
		}
	}
	return 0
}

// isUpgrade reports whether latest is a strict version upgrade over installed.
// Both must be non-empty: a plugin that declares no version can never report an
// update (nothing to compare), which is how we avoid a permanent false "update".
func isUpgrade(installed, latest string) bool {
	if strings.TrimSpace(installed) == "" || strings.TrimSpace(latest) == "" {
		return false
	}
	return CompareVersions(latest, installed) > 0
}

func splitVersion(v string) []string {
	v = strings.TrimSpace(v)
	v = strings.TrimPrefix(v, "v")
	v = strings.TrimPrefix(v, "V")
	if i := strings.IndexAny(v, "-+"); i >= 0 { // drop prerelease / build metadata
		v = v[:i]
	}
	if v == "" {
		return nil
	}
	return strings.Split(v, ".")
}

// checkUpdatesConcurrency bounds how many install sources we re-fetch at once
// during an update check — each is a network download of a plugin tarball.
const checkUpdatesConcurrency = 6

// CheckUpdates re-fetches the manifest at each installed plugin's install source
// and reports whether a newer version is published. Plugins with no recorded
// source (bundled examples, hand-dropped dirs) are returned with HasUpdate=false
// and no network call — there is nothing to update from. Network fetches run
// concurrently (bounded) so a manager with many plugins doesn't check serially.
func CheckUpdates(installed []Manifest) []UpdateStatus {
	results := make([]UpdateStatus, len(installed))
	var wg sync.WaitGroup
	sem := make(chan struct{}, checkUpdatesConcurrency)
	for i := range installed {
		m := installed[i]
		if strings.TrimSpace(m.Source) == "" {
			results[i] = UpdateStatus{ID: m.ID, Installed: m.Version}
			continue
		}
		wg.Add(1)
		sem <- struct{}{}
		go func(i int, m Manifest) {
			defer wg.Done()
			defer func() { <-sem }()
			st := UpdateStatus{ID: m.ID, Installed: m.Version}
			remote, err := Inspect(m.Source)
			if err != nil {
				st.Error = err.Error()
			} else {
				st.Latest = remote.Version
				st.HasUpdate = isUpgrade(m.Version, remote.Version)
			}
			results[i] = st
		}(i, m)
	}
	wg.Wait()
	return results
}
