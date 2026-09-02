package main

// usage.pollOnBoot, read from the config document the desktop and the headless
// brain already share (~/.config/workspacer/config.yaml, defaults in
// cmd/brain/config_defaults.json).
//
// This launcher has no config service of its own and does not need one: the
// single key it must forward is a boolean, and reading it here keeps the
// headless spawn site honouring the same setting the desktop's
// claudemonDaemon.ts does. Everything else about the document — deep-merge,
// migrations, the mtime gate — belongs to the two real writers and is
// deliberately not duplicated.

import (
	"os"
	"path/filepath"

	yaml "gopkg.in/yaml.v3"
)

// usagePollOnBootSetting returns the operator's `usage.pollOnBoot` choice, or
// nil when the config says nothing about it.
//
// nil is not false. An unreadable file, an absent section and a non-boolean
// value all mean "unstated", and the caller must leave the environment
// variable unset for those — claudemon reads an absent variable as ON, which
// is the shipped default. Answering false on a parse failure would silently
// switch a feature off because a YAML document had a tab in it.
func usagePollOnBootSetting(dir string) *bool {
	raw, err := os.ReadFile(filepath.Join(dir, "config.yaml"))
	if err != nil {
		return nil
	}
	var doc map[string]any
	if err := yaml.Unmarshal(raw, &doc); err != nil {
		return nil
	}
	usage, ok := doc["usage"].(map[string]any)
	if !ok {
		return nil
	}
	v, ok := usage["pollOnBoot"].(bool)
	if !ok {
		return nil
	}
	return &v
}
