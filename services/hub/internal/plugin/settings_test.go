package plugin

import (
	"encoding/json"
	"testing"
)

func TestSettingsValidation(t *testing.T) {
	base := func(s ...SettingDef) Manifest {
		return Manifest{ID: "x", APIVersion: APIVersion, Settings: s}
	}

	ok := base(
		SettingDef{Key: "vimMode", Label: "Vim mode", Type: SettingBoolean, Default: false},
		SettingDef{Key: "tabSize", Label: "Tab size", Type: SettingNumber, Default: 2},
		SettingDef{Key: "theme", Label: "Theme", Type: SettingSelect, Options: []string{"dark", "light"}, Default: "dark"},
	)
	if err := ok.Validate(); err != nil {
		t.Fatalf("valid settings rejected: %v", err)
	}

	cases := []struct {
		name string
		mf   Manifest
	}{
		{"empty key", base(SettingDef{Type: SettingBoolean})},
		{"unknown type", base(SettingDef{Key: "k", Type: "color"})},
		{"select without options", base(SettingDef{Key: "k", Type: SettingSelect})},
		{"duplicate key", base(
			SettingDef{Key: "k", Type: SettingBoolean},
			SettingDef{Key: "k", Type: SettingString},
		)},
	}
	for _, c := range cases {
		if err := c.mf.Validate(); err == nil {
			t.Errorf("%s: expected validation error", c.name)
		}
	}
}

func TestSettingsUnmarshal(t *testing.T) {
	var caps []SettingDef
	in := `[
	  {"key":"vimMode","label":"Vim mode","type":"boolean","default":false},
	  {"key":"tabSize","label":"Tab size","type":"number","default":2,"help":"Spaces per indent"},
	  {"key":"wrap","label":"Soft wrap","type":"select","options":["off","on"],"default":"off"}
	]`
	if err := json.Unmarshal([]byte(in), &caps); err != nil {
		t.Fatal(err)
	}
	if len(caps) != 3 || caps[0].Key != "vimMode" || caps[2].Options[1] != "on" {
		t.Fatalf("unexpected parse: %+v", caps)
	}
}
