package plugin

import "testing"

func TestCompareVersions(t *testing.T) {
	cases := []struct {
		a, b string
		want int
	}{
		{"1.0.0", "1.0.0", 0},
		{"1.2", "1.2.0", 0},   // missing trailing segment == 0
		{"v1.2.0", "1.2.0", 0}, // leading v ignored
		{"1.2.0", "1.3.0", -1},
		{"1.3.0", "1.2.0", 1},
		{"1.2.0", "1.10.0", -1}, // numeric, not lexical (2 < 10)
		{"2.0.0", "1.9.9", 1},
		{"1.2.0-rc1", "1.2.0", 0}, // prerelease suffix dropped
		{"1.2.0+build7", "1.2.0", 0},
		{"0.1.0", "0.2.0", -1},
		{"", "", 0},
	}
	for _, c := range cases {
		if got := CompareVersions(c.a, c.b); got != c.want {
			t.Errorf("CompareVersions(%q, %q) = %d, want %d", c.a, c.b, got, c.want)
		}
	}
}

func TestIsUpgrade(t *testing.T) {
	cases := []struct {
		installed, latest string
		want              bool
	}{
		{"1.0.0", "1.1.0", true},
		{"1.1.0", "1.1.0", false},
		{"1.2.0", "1.1.0", false}, // downgrade is not an upgrade
		{"", "1.0.0", false},      // no installed version → never an update
		{"1.0.0", "", false},      // no published version → never an update
		{"", "", false},
	}
	for _, c := range cases {
		if got := isUpgrade(c.installed, c.latest); got != c.want {
			t.Errorf("isUpgrade(%q, %q) = %v, want %v", c.installed, c.latest, got, c.want)
		}
	}
}

// CheckUpdates must not hit the network for plugins without an install source,
// and must preserve input order in its results.
func TestCheckUpdatesNoSource(t *testing.T) {
	in := []Manifest{
		{ID: "a", Version: "1.0.0"},              // no source
		{ID: "b", Version: "2.0.0", Source: ""},  // explicit empty source
	}
	got := CheckUpdates(in)
	if len(got) != 2 {
		t.Fatalf("got %d statuses, want 2", len(got))
	}
	for i, st := range got {
		if st.ID != in[i].ID {
			t.Errorf("result[%d].ID = %q, want %q (order not preserved)", i, st.ID, in[i].ID)
		}
		if st.HasUpdate {
			t.Errorf("result[%d] (no source) reports HasUpdate", i)
		}
		if st.Error != "" {
			t.Errorf("result[%d] (no source) errored: %s", i, st.Error)
		}
	}
}
