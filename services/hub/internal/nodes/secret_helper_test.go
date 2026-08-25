package nodes

import (
	"encoding/json"
	"strings"
)

// containsSecret renders a view exactly as the bus would and looks for the
// credential in the bytes. Rendering rather than field-checking is deliberate:
// it catches a leak through any field, including one added later.
func containsSecret(v NodeView) bool {
	raw, err := json.Marshal(v)
	if err != nil {
		return true // unrenderable is not a pass
	}
	return strings.Contains(string(raw), secretToken)
}
