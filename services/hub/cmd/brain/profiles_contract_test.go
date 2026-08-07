package main

// The claude.profiles.* contract, shared with apps/desktop's claudeProfiles.ts.
// See contracts/claude-profiles-cases.json for what diverged and why.

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

type profilesContract struct {
	List []struct {
		Name         string     `json:"name"`
		File         *[]profile `json:"file"`
		ExpectedList []profile  `json:"expectedList"`
		ExpectedFile []profile  `json:"expectedFile"`
		Why          string     `json:"why"`
	} `json:"list"`
	Add []struct {
		Name string     `json:"name"`
		File *[]profile `json:"file"`
		Add  struct {
			Name       string   `json:"name"`
			ConfigDir  string   `json:"configDir"`
			ExtraArgs  []string `json:"extraArgs"`
			MCPItemIDs []string `json:"mcpItemIds"`
		} `json:"add"`
		ExpectedAdded profile  `json:"expectedAdded"`
		ExpectedIDs   []string `json:"expectedFileIds"`
		Why           string   `json:"why"`
	} `json:"add"`
	Mutate []struct {
		Name        string                 `json:"name"`
		File        *[]profile             `json:"file"`
		UpdateID    string                 `json:"updateId"`
		Update      *struct{ Name string } `json:"update"`
		ExpectFound bool                   `json:"expectFound"`
		RemoveID    string                 `json:"removeId"`
		ExpectedIDs []string               `json:"expectedFileIds"`
		Why         string                 `json:"why"`
	} `json:"mutate"`
}

func loadProfilesContract(t *testing.T) profilesContract {
	t.Helper()
	path := filepath.Join("..", "..", "..", "..", "contracts", "claude-profiles-cases.json")
	raw, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read %s: %v", path, err)
	}
	var fx profilesContract
	if err := json.Unmarshal(raw, &fx); err != nil {
		t.Fatalf("parse %s: %v", path, err)
	}
	if len(fx.List) == 0 || len(fx.Add) == 0 || len(fx.Mutate) == 0 {
		t.Fatal("the fixture is missing a block; this loader guards nothing")
	}
	return fx
}

// seedProfiles points the config dir at a fresh sandbox and writes `file`.
func seedProfiles(t *testing.T, file *[]profile) {
	t.Helper()
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	t.Setenv("APPDATA", t.TempDir())
	if file == nil {
		return
	}
	if err := os.MkdirAll(configDir(), 0o755); err != nil {
		t.Fatal(err)
	}
	data, _ := json.Marshal(map[string][]profile{"profiles": *file})
	if err := os.WriteFile(profilesPath(), data, 0o644); err != nil {
		t.Fatal(err)
	}
}

func profileIDs(ps []profile) []string {
	out := make([]string, 0, len(ps))
	for _, p := range ps {
		out = append(out, p.ID)
	}
	return out
}

func TestClaudeProfilesContractCases(t *testing.T) {
	fx := loadProfilesContract(t)

	for _, c := range fx.List {
		t.Run("list/"+c.Name, func(t *testing.T) {
			seedProfiles(t, c.File)
			gotJSON, _ := json.Marshal(loadProfiles())
			wantJSON, _ := json.Marshal(c.ExpectedList)
			if string(gotJSON) != string(wantJSON) {
				t.Errorf("claude.profiles.list drifted\n  got:  %s\n  want: %s\n  why:  %s", gotJSON, wantJSON, c.Why)
			}
			onDisk, _ := json.Marshal(readProfilesFile())
			wantDisk, _ := json.Marshal(c.ExpectedFile)
			if string(onDisk) != string(wantDisk) {
				t.Errorf("the profiles FILE drifted\n  got:  %s\n  want: %s", onDisk, wantDisk)
			}
		})
	}

	for _, c := range fx.Add {
		t.Run("add/"+c.Name, func(t *testing.T) {
			seedProfiles(t, c.File)
			got, err := addProfile(c.Add.Name, c.Add.ConfigDir, c.Add.ExtraArgs, c.Add.MCPItemIDs)
			if err != nil {
				t.Fatalf("add: %v", err)
			}
			if got.ID == "" {
				t.Fatal("add returned a profile with no id")
			}
			want := c.ExpectedAdded
			want.ID = got.ID // the uuid is not part of the contract
			gotJSON, _ := json.Marshal(got)
			wantJSON, _ := json.Marshal(&want)
			if string(gotJSON) != string(wantJSON) {
				t.Errorf("claude.profiles.add drifted\n  got:  %s\n  want: %s\n  why:  %s", gotJSON, wantJSON, c.Why)
			}
			ids := profileIDs(readProfilesFile())
			if len(ids) != len(c.ExpectedIDs) {
				t.Fatalf("stored ids = %v, want %v", ids, c.ExpectedIDs)
			}
			for i, want := range c.ExpectedIDs {
				if want == "<added>" {
					want = got.ID
				}
				if ids[i] != want {
					t.Errorf("stored id[%d] = %q, want %q (%v)", i, ids[i], want, ids)
				}
			}
		})
	}

	for _, c := range fx.Mutate {
		t.Run("mutate/"+c.Name, func(t *testing.T) {
			seedProfiles(t, c.File)
			if c.UpdateID != "" {
				_ = loadProfiles() // the caller has listed first, as a client would
				name := c.Update.Name
				got, err := updateProfile(c.UpdateID, profileUpdate{Name: &name})
				if c.ExpectFound && err != nil {
					t.Fatalf("update(%q) failed for an id list() returned: %v — %s", c.UpdateID, err, c.Why)
				}
				if c.ExpectFound && got.Name != name {
					t.Errorf("update did not apply: %+v", got)
				}
			}
			if c.RemoveID != "" {
				if err := removeProfile(c.RemoveID); err != nil {
					t.Fatal(err)
				}
			}
			if c.ExpectedIDs != nil {
				if ids := profileIDs(readProfilesFile()); len(ids) != len(c.ExpectedIDs) {
					t.Fatalf("stored ids = %v, want %v — %s", ids, c.ExpectedIDs, c.Why)
				}
			}
		})
	}
}
