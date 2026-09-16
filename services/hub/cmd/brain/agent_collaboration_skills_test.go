package main

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestHeadlessAgentCollaborationSkillsInstallImmutablePointerOnly(t *testing.T) {
	cwd := t.TempDir()
	note := installHeadlessAgentCollaborationSkills("codex", cwd, false)
	root := filepath.Join(cwd, ".workspacer", "skills", headlessAgentCollaborationSkillsVersion)
	if !strings.Contains(note, filepath.Join(root, "spawn-agent", "SKILL.md")) ||
		!strings.Contains(note, filepath.Join(root, "project-brief", "SKILL.md")) {
		t.Fatalf("pointer note = %q", note)
	}
	if strings.Contains(note, "# Spawn an agent") || strings.Contains(note, "---\nname:") {
		t.Fatalf("prompt copied skill bodies instead of pointers: %q", note)
	}
	for rel, want := range headlessAgentCollaborationSkillFiles {
		got, err := os.ReadFile(filepath.Join(root, filepath.FromSlash(rel)))
		if err != nil || string(got) != want {
			t.Fatalf("installed %s mismatch: err=%v", rel, err)
		}
	}
	var encoded bytes.Buffer
	encoder := json.NewEncoder(&encoded)
	encoder.SetEscapeHTML(false)
	if err := encoder.Encode(headlessAgentCollaborationSkillFiles); err != nil {
		t.Fatal(err)
	}
	body := bytes.TrimSuffix(encoded.Bytes(), []byte("\n"))
	sum := sha256.Sum256(body)
	if got := hex.EncodeToString(sum[:])[:16]; got != headlessAgentCollaborationSkillsVersion {
		t.Fatalf("version = %s, want generated asset hash %s", headlessAgentCollaborationSkillsVersion, got)
	}
}

func TestHeadlessAgentCollaborationSkillsExcludeManagersAndPi(t *testing.T) {
	for _, tc := range []struct {
		provider string
		manager  bool
	}{{"codex", true}, {"pi", false}} {
		cwd := t.TempDir()
		if got := installHeadlessAgentCollaborationSkills(tc.provider, cwd, tc.manager); got != "" {
			t.Fatalf("provider=%s manager=%v got %q", tc.provider, tc.manager, got)
		}
		if _, err := os.Stat(filepath.Join(cwd, ".workspacer")); !os.IsNotExist(err) {
			t.Fatalf("excluded session installed skills: %v", err)
		}
	}
}

func TestHeadlessAgentCollaborationSkillsRefuseUnsafeRootsAndCollisions(t *testing.T) {
	t.Run("home", func(t *testing.T) {
		cwd := t.TempDir()
		t.Setenv("HOME", cwd)
		if got := installHeadlessAgentCollaborationSkills("codex", cwd, false); got != "" {
			t.Fatalf("home accepted: %q", got)
		}
	})
	t.Run("symlink cwd", func(t *testing.T) {
		real := t.TempDir()
		link := filepath.Join(t.TempDir(), "project")
		if err := os.Symlink(real, link); err != nil {
			t.Fatal(err)
		}
		if got := installHeadlessAgentCollaborationSkills("codex", link, false); got != "" {
			t.Fatalf("symlink cwd accepted: %q", got)
		}
	})
	t.Run("user collision", func(t *testing.T) {
		cwd := t.TempDir()
		file := filepath.Join(cwd, ".workspacer", "skills", headlessAgentCollaborationSkillsVersion, "spawn-agent", "SKILL.md")
		if err := os.MkdirAll(filepath.Dir(file), 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(file, []byte("user owned\n"), 0o644); err != nil {
			t.Fatal(err)
		}
		if got := installHeadlessAgentCollaborationSkills("codex", cwd, false); got != "" {
			t.Fatalf("collision accepted: %q", got)
		}
		got, _ := os.ReadFile(file)
		if string(got) != "user owned\n" {
			t.Fatalf("collision overwritten: %q", got)
		}
	})
}

func TestHeadlessAgentCollaborationSkillsRemoveOnlyExactLegacyCopies(t *testing.T) {
	cwd := t.TempDir()
	exact := filepath.Join(cwd, ".agents", "skills", "spawn-agent", "SKILL.md")
	user := filepath.Join(cwd, ".agents", "skills", "project-brief", "SKILL.md")
	if err := os.MkdirAll(filepath.Dir(exact), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(filepath.Dir(user), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(exact, []byte(headlessAgentCollaborationSkillFiles["spawn-agent/SKILL.md"]), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(user, []byte("custom"), 0o644); err != nil {
		t.Fatal(err)
	}
	_ = installHeadlessAgentCollaborationSkills("codex", cwd, false)
	if _, err := os.Stat(exact); !os.IsNotExist(err) {
		t.Fatalf("exact legacy copy remains: %v", err)
	}
	if got, _ := os.ReadFile(user); string(got) != "custom" {
		t.Fatalf("custom legacy copy changed: %q", got)
	}
}
