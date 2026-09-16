package main

// Tests for the path behavior that remains active after workspace and secret
// grants were removed: absolute-path canonicalization and explicit containment
// of paths derived inside a selected semantic object.

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
)

const contractFixtureRel = "contracts/path-containment-cases.json"

// Compatibility message still asserted by legacy anonymous-policy tests. It is
// not the authenticated agent/plugin policy.
const refusalText = "path is outside the allowed workspace (agent cwds + config stores)"

const sessionFilenameFloor = 12

func readContractFixtureBytes(t *testing.T) []byte {
	t.Helper()
	return mustReadRepoFile(t, "contracts", "path-containment-cases.json")
}

// contractTree is shared by the filename and library-item contract loaders.
type contractTree struct {
	Dirs             []string          `json:"dirs"`
	Files            map[string]string `json:"files"`
	Symlinks         map[string]string `json:"symlinks"`
	RelativeSymlinks map[string]string `json:"relativeSymlinks"`
	Modes            map[string]string `json:"modes"`
}

func contractTokenRefs(s string) (names []string, unterminated bool) {
	for i := 0; i < len(s); {
		j := strings.Index(s[i:], "${")
		if j < 0 {
			break
		}
		start := i + j + 2
		end := strings.IndexByte(s[start:], '}')
		if end < 0 {
			return names, true
		}
		names = append(names, s[start:start+end])
		i = start + end + 1
	}
	return names, false
}

// libraryCwdWithConfigDir builds two distinct semantic objects: a project
// library and host configuration state. Selected-library tests use it to prove
// a derived item cannot switch objects through a symlink.
func libraryCwdWithConfigDir(t *testing.T) (cwd, token string) {
	t.Helper()
	sandbox, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	t.Setenv("XDG_CONFIG_HOME", filepath.Join(sandbox, "config"))
	t.Setenv("APPDATA", filepath.Join(sandbox, "config"))
	if err := os.MkdirAll(configDir(), 0o755); err != nil {
		t.Fatal(err)
	}
	token = filepath.Join(configDir(), "remote-token")
	if err := os.WriteFile(token, []byte("HOST-CONFIG-MARKER"), 0o600); err != nil {
		t.Fatal(err)
	}
	cwd = filepath.Join(sandbox, "project")
	if err := os.MkdirAll(cwd, 0o755); err != nil {
		t.Fatal(err)
	}
	return cwd, token
}

type activePathCase struct {
	Name          string       `json:"name"`
	Group         string       `json:"group"`
	Expect        string       `json:"expect"`
	Why           string       `json:"why"`
	Roots         []string     `json:"roots"`
	Target        string       `json:"target"`
	DeniedBy      string       `json:"deniedBy"`
	ResolvesTo    string       `json:"resolvesTo"`
	NeedsSymlinks bool         `json:"needsSymlinks"`
	Tree          contractTree `json:"tree"`
}

// TestActivePathContractCases is the Go loader for the active cross-language
// path contract. It intentionally tests ambient canonicalization separately
// from containment inside a selected semantic object.
func TestActivePathContractCases(t *testing.T) {
	var fx struct {
		Cases []activePathCase `json:"cases"`
	}
	if err := json.Unmarshal(readContractFixtureBytes(t), &fx); err != nil {
		t.Fatal(err)
	}
	if len(fx.Cases) < 7 {
		t.Fatalf("active path contract has %d cases, want at least 7", len(fx.Cases))
	}
	for _, c := range fx.Cases {
		t.Run(c.Name, func(t *testing.T) {
			sandbox, err := filepath.EvalSymlinks(t.TempDir())
			if err != nil {
				t.Fatal(err)
			}
			root := filepath.Join(sandbox, "root")
			outside := filepath.Join(sandbox, "outside")
			if err := os.MkdirAll(root, 0o755); err != nil {
				t.Fatal(err)
			}
			if err := os.MkdirAll(outside, 0o755); err != nil {
				t.Fatal(err)
			}
			sub := func(s string) string {
				s = strings.ReplaceAll(s, "${SANDBOX}", sandbox)
				s = strings.ReplaceAll(s, "${ROOT}", root)
				return strings.ReplaceAll(s, "${OUTSIDE}", outside)
			}
			for _, dir := range c.Tree.Dirs {
				if err := os.MkdirAll(filepath.Join(sandbox, filepath.FromSlash(dir)), 0o755); err != nil {
					t.Fatal(err)
				}
			}
			for link, target := range c.Tree.Symlinks {
				if runtime.GOOS == "windows" {
					t.Skip("symlink privilege is not portable on Windows")
				}
				linkPath := filepath.Join(sandbox, filepath.FromSlash(link))
				if err := os.MkdirAll(filepath.Dir(linkPath), 0o755); err != nil {
					t.Fatal(err)
				}
				if err := os.Symlink(filepath.Join(sandbox, filepath.FromSlash(target)), linkPath); err != nil {
					t.Skipf("symlinks unavailable: %v", err)
				}
			}
			roots := make([]string, len(c.Roots))
			for i, value := range c.Roots {
				roots[i] = sub(value)
			}
			target := sub(c.Target)
			var got string
			if c.Group == "selected-object" {
				got, err = assertPathContained("contract", target, roots)
			} else {
				got, err = assertPathAllowed("contract", target, roots)
			}
			if c.Expect == "deny" {
				if err == nil {
					t.Fatalf("expected refusal, got %q\nwhy: %s", got, c.Why)
				}
				return
			}
			if err != nil {
				t.Fatalf("unexpected refusal: %v\nwhy: %s", err, c.Why)
			}
			if want := sub(c.ResolvesTo); got != want {
				t.Fatalf("resolved %q, want %q\nwhy: %s", got, want, c.Why)
			}
		})
	}
}

// registryWithCwds remains a general test fixture for handlers that derive
// project state from live sessions. It no longer describes an authorization
// root set.
func registryWithCwd(t *testing.T, dir string) *registry {
	t.Helper()
	return registryWithCwds(t, dir)
}

func registryWithCwds(t *testing.T, dirs ...string) *registry {
	t.Helper()
	store := newSessionStore()
	for i, dir := range dirs {
		id := string(rune('a' + i))
		snap, err := json.Marshal(map[string]any{
			"session_id": id,
			"cwd":        dir,
			"mode":       "input",
		})
		if err != nil {
			t.Fatal(err)
		}
		store.set(id, snap)
	}
	r := newRegistry(newClaudemonClient("http://127.0.0.1:0"))
	r.store = store
	return r
}

func TestAuthenticatedAgentPathsAreAmbientButCanonical(t *testing.T) {
	selected := t.TempDir()
	outside := t.TempDir()
	target := filepath.Join(outside, "missing", "file.txt")
	got, err := assertPathAllowed("fs.read", target, []string{selected})
	if err != nil {
		t.Fatalf("ambient absolute path rejected: %v", err)
	}
	want, err := canonicalizePath(target)
	if err != nil {
		t.Fatal(err)
	}
	if got != want {
		t.Fatalf("opened path = %q, canonical path = %q", got, want)
	}
	if _, err := assertPathAllowed("fs.read", "relative/file.txt", nil); err == nil {
		t.Fatal("relative path was accepted")
	}
}

func TestCanonicalizationResolvesSymlinkBeforeParentTraversal(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("symlink privilege is not portable on Windows")
	}
	sandbox, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	root := filepath.Join(sandbox, "root")
	outside := filepath.Join(sandbox, "outside", "nested")
	if err := os.MkdirAll(root, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(outside, 0o755); err != nil {
		t.Fatal(err)
	}
	link := filepath.Join(root, "link")
	if err := os.Symlink(outside, link); err != nil {
		t.Skipf("symlinks unavailable: %v", err)
	}
	got, err := canonicalizePath(link + string(filepath.Separator) + ".." + string(filepath.Separator) + "value.txt")
	if err != nil {
		t.Fatal(err)
	}
	want := filepath.Join(sandbox, "outside", "value.txt")
	if got != want {
		t.Fatalf("canonical path = %q, want %q", got, want)
	}
}

func TestSelectedObjectContainmentRemainsExplicit(t *testing.T) {
	selected := t.TempDir()
	inside := filepath.Join(selected, "nested", "item.md")
	outside := filepath.Join(t.TempDir(), "item.md")
	if _, err := assertPathContained("library.read", inside, []string{selected}); err != nil {
		t.Fatalf("selected-object child rejected: %v", err)
	}
	if _, err := assertPathContained("library.read", outside, []string{selected}); err == nil {
		t.Fatal("path outside the selected object was accepted")
	}
}

func TestMaxLinkHopsMatchesTheContract(t *testing.T) {
	var fx struct {
		MaxLinkHops int `json:"maxLinkHops"`
	}
	if err := json.Unmarshal(readContractFixtureBytes(t), &fx); err != nil {
		t.Fatal(err)
	}
	if fx.MaxLinkHops == 0 || fx.MaxLinkHops != maxLinkHops {
		t.Fatalf("maxLinkHops = %d, contract = %d", maxLinkHops, fx.MaxLinkHops)
	}
}

func TestAmbientFilesystemHandlersIgnoreFormerWorkspaceRoots(t *testing.T) {
	root := t.TempDir()
	outside := filepath.Join(t.TempDir(), "note.txt")
	if err := os.WriteFile(outside, []byte("ambient"), 0o600); err != nil {
		t.Fatal(err)
	}
	r := registryWithCwd(t, root)
	got, err := r.handle(context.Background(), "fs.read", json.RawMessage(`{"path":`+jsonStr(outside)+`}`))
	if err != nil {
		t.Fatalf("fs.read outside the session cwd: %v", err)
	}
	if string(got) == "" {
		t.Fatal("fs.read returned an empty result")
	}
}
