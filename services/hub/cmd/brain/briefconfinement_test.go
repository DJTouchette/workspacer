package main

// THE CONFINEMENT ON THE TWO NEW PATH-BEARING CAPABILITIES — brief.append and
// fs.readImage — driven through the real dispatch, with a planted secret.
//
// The corpus sweep (TestEveryPathBearingBrainMethodIsConfined) now covers both,
// and that is the systematic half. This is the specific half, in the shape the
// git port established: attempt the thing the guard exists to prevent, assert
// the refusal AND that nothing leaked, and keep a control case in each block so
// a handler that refuses unconditionally cannot pass.
//
// The two have DIFFERENT confinement stories and each is checked on its own
// terms:
//
//   - brief.append WRITES. Its caller-chosen value is a DIRECTORY, and both
//     path components under it are provider literals. So the escape to hunt is
//     not "name a file outside" — the caller cannot name a file at all — but
//     "name a directory outside", and the subtler one: a directory that PASSES
//     the guard and then resolves elsewhere, which is what a symlinked
//     .workspacer inside an allowed project would do if the brief path were
//     composed from the caller's string instead of the guard's answer.
//   - fs.readImage READS BYTES BACK. The path guard is only half of it: within
//     an allowed root, the extension allowlist is what stops a method named
//     "read image" from being a general-purpose file reader that returns the
//     bytes base64'd in a data: URL. Both halves are tested, because either one
//     alone leaks.

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
)

// briefFixture is one sandbox: an allowed agent cwd, a sibling project that is
// NOT a root, and an outside tree — plus a planted secret in each place the
// guard has to keep a caller out of.
type briefFixture struct {
	sandbox  string
	agentCwd string // the ONE live agent cwd, and therefore the only workspace root
	sibling  string // <sandbox>/other-project — a real project, not this caller's
	outside  string // <sandbox>/outside
}

func newBriefFixture(t *testing.T) briefFixture {
	t.Helper()
	root, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	setHome(t, filepath.Join(root, "home"))
	setConfigHome(t, filepath.Join(root, "config"))
	resetCwdCacheForTest()
	t.Cleanup(resetCwdCacheForTest)

	fx := briefFixture{
		sandbox:  root,
		agentCwd: filepath.Join(root, "project"),
		sibling:  filepath.Join(root, "other-project"),
		outside:  filepath.Join(root, "outside"),
	}
	for _, d := range []string{
		filepath.Join(root, "home"),
		fx.agentCwd,
		filepath.Join(fx.sibling, ".workspacer"),
		fx.outside,
	} {
		if err := os.MkdirAll(d, 0o755); err != nil {
			t.Fatal(err)
		}
	}
	write := func(p, contents string) {
		t.Helper()
		if err := os.WriteFile(p, []byte(contents), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	// The secret, in every place a caller must not reach it from.
	write(filepath.Join(fx.outside, "secret.env"), secretMarker+"\n")
	write(filepath.Join(fx.sibling, ".workspacer", "brief.md"), "## Recently\n- "+secretMarker+"\n")
	// A "photo" outside the roots, and a secret INSIDE them — the first tests
	// the path guard, the second tests the extension allowlist.
	write(filepath.Join(fx.outside, "photo.png"), secretMarker+"\n")
	write(filepath.Join(fx.agentCwd, ".env"), secretMarker+"\n")
	write(filepath.Join(fx.agentCwd, "id_rsa"), secretMarker+"\n")
	return fx
}

func (fx briefFixture) call(t *testing.T, method string, params map[string]any) (string, error) {
	t.Helper()
	reg := registryWithCwds(t, fx.agentCwd)
	body, err := json.Marshal(params)
	if err != nil {
		t.Fatal(err)
	}
	res, err := reg.handle(context.Background(), method, json.RawMessage(body))
	return string(res), err
}

// mustRefuse asserts the ONE containment refusal, and separately that nothing
// leaked. A call that failed for an unrelated reason (a decode error, ENOENT)
// proves nothing about confinement, which is why the message is checked.
func (fx briefFixture) mustRefuse(t *testing.T, method string, params map[string]any) {
	t.Helper()
	res, err := fx.call(t, method, params)
	if err == nil {
		t.Fatalf("%s %v was ALLOWED — response: %s", method, params, truncate(res))
	}
	if !strings.Contains(err.Error(), refusalText) {
		t.Fatalf("%s %v was rejected for the wrong reason: %v", method, params, err)
	}
	if strings.Contains(res, secretMarker) || strings.Contains(err.Error(), secretMarker) {
		t.Fatalf("%s %v LEAKED the secret despite refusing: %s / %v", method, params, truncate(res), err)
	}
}

// ── brief.append: the caller names a DIRECTORY ──────────────────────────────

func TestBriefAppendCannotWriteOutsideTheAllowedRoots(t *testing.T) {
	fx := newBriefFixture(t)

	for _, c := range []struct{ name, project string }{
		{"a plain directory outside every root", fx.outside},
		{"another project on the same machine", fx.sibling},
		{"the sandbox itself, which CONTAINS the allowed root", fx.sandbox},
		{"the home directory", filepath.Join(fx.sandbox, "home")},
		{"a traversal out of the allowed root", filepath.Join(fx.agentCwd, "..", "outside")},
		{"a traversal dressed as a subdirectory", filepath.Join(fx.agentCwd, "sub", "..", "..", "outside")},
	} {
		t.Run(c.name, func(t *testing.T) {
			fx.mustRefuse(t, "brief.append", map[string]any{
				"project": c.project, "section": "Recently", "line": "planted by a bus caller",
			})
			// Nothing was written where it was aimed. A refusal that still
			// created the file would be the whole failure, reported as a
			// success only to whoever looked at the disk.
			if _, err := os.Stat(briefPathFor(c.project)); err == nil {
				body, _ := os.ReadFile(briefPathFor(c.project))
				if strings.Contains(string(body), "planted by a bus caller") {
					t.Fatalf("a REFUSED brief.append still wrote to %s", briefPathFor(c.project))
				}
			}
		})
	}

	// THE CONTROL. The guard must not simply deny everything: the caller's OWN
	// project is allowed, or every case above passes for the wrong reason.
	res, err := fx.call(t, "brief.append", map[string]any{
		"project": fx.agentCwd, "section": "Recently", "line": "legitimate line",
	})
	if err != nil {
		t.Fatalf("the allowed project was REFUSED, so every denial above proves nothing: %v", err)
	}
	if !strings.Contains(res, "legitimate line") {
		t.Errorf("the allowed append did not report the line it wrote: %s", truncate(res))
	}
	body, err := os.ReadFile(briefPathFor(fx.agentCwd))
	if err != nil || !strings.Contains(string(body), "- legitimate line") {
		t.Fatalf("the allowed append did not land on disk: %v / %s", err, body)
	}
}

// THE SUBTLE ONE, and the reason the brief path is composed under the guard's
// CANONICAL answer rather than under the caller's string.
//
// `project` passes the guard — it IS the allowed agent cwd. But the directory
// the brief lives in, <project>/.workspacer, is a symlink pointing out of every
// root. Composing the path from the caller's string and writing it would follow
// that symlink; resolving the guard's answer and re-asserting does not. This is
// the same check-path/opened-path split the git port had to carry.
func TestBriefAppendDoesNotFollowASymlinkedWorkspacerDirectory(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("symlink creation needs elevation on Windows; the POSIX runs cover the case")
	}
	fx := newBriefFixture(t)
	link := filepath.Join(fx.agentCwd, ".workspacer")
	if err := os.Symlink(fx.outside, link); err != nil {
		t.Skipf("symlinks unavailable here: %v", err)
	}

	res, err := fx.call(t, "brief.append", map[string]any{
		"project": fx.agentCwd, "section": "Recently", "line": "planted through a symlink",
	})

	escaped := filepath.Join(fx.outside, "brief.md")
	if body, readErr := os.ReadFile(escaped); readErr == nil && strings.Contains(string(body), "planted through a symlink") {
		t.Fatalf("brief.append wrote OUTSIDE every allowed root by following a symlinked .workspacer: %s\n(call returned %s / %v)",
			escaped, truncate(res), err)
	}
}

// ── fs.readImage: the path guard AND the extension allowlist ────────────────

func TestReadImageCannotEscapeTheAllowedRoots(t *testing.T) {
	fx := newBriefFixture(t)
	for _, c := range []struct{ name, path string }{
		{"an image outside every root", filepath.Join(fx.outside, "photo.png")},
		{"another project's tree", filepath.Join(fx.sibling, ".workspacer", "brief.md")},
		{"a traversal out of the allowed root", filepath.Join(fx.agentCwd, "..", "outside", "photo.png")},
	} {
		t.Run(c.name, func(t *testing.T) {
			fx.mustRefuse(t, "fs.readImage", map[string]any{"path": c.path})
		})
	}
}

// THE SECOND HALF, and the one a path guard alone does not close: inside an
// allowed root, only allowlisted extensions are served. Without it, fs.readImage
// returns the bytes of any file an agent's cwd contains, base64'd into a data:
// URL — a general-purpose file reader wearing the name of a thumbnailer.
func TestReadImageRefusesNonImagesInsideTheAllowedRoots(t *testing.T) {
	fx := newBriefFixture(t)
	for _, name := range []string{".env", "id_rsa"} {
		t.Run(name, func(t *testing.T) {
			target := filepath.Join(fx.agentCwd, name)
			// This file IS inside the allowed root — the path guard passes it —
			// so anything that refuses here is the allowlist doing the work.
			if _, err := assertPathAllowed("probe", target, []string{fx.agentCwd}); err != nil {
				t.Fatalf("fixture is wrong: %s is not inside the allowed root (%v)", target, err)
			}
			res, err := fx.call(t, "fs.readImage", map[string]any{"path": target})
			if err == nil {
				t.Fatalf("fs.readImage served %s from inside an allowed root — response: %s", name, truncate(res))
			}
			if strings.Contains(res, secretMarker) {
				t.Fatalf("fs.readImage LEAKED %s despite refusing: %s", name, truncate(res))
			}
			// Base64 too: the leak this method could produce is encoded, so a
			// plaintext grep alone would not see it.
			if strings.Contains(res, base64Of(secretMarker)) {
				t.Fatalf("fs.readImage leaked %s as base64: %s", name, truncate(res))
			}
		})
	}

	// THE CONTROL: a real image inside the allowed root is served, so the
	// refusals above are the allowlist and not a handler that fails at everything.
	png := filepath.Join(fx.agentCwd, "shot.png")
	if err := os.WriteFile(png, onePixelPNG(), 0o600); err != nil {
		t.Fatal(err)
	}
	res, err := fx.call(t, "fs.readImage", map[string]any{"path": png})
	if err != nil {
		t.Fatalf("an allowed image was REFUSED, so every denial above proves nothing: %v", err)
	}
	if !strings.Contains(res, "data:image/png;base64,") {
		t.Errorf("the allowed image did not come back as a data URL: %s", truncate(res))
	}
}

func base64Of(s string) string {
	const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
	var out strings.Builder
	b := []byte(s)
	for i := 0; i+2 < len(b); i += 3 {
		n := int(b[i])<<16 | int(b[i+1])<<8 | int(b[i+2])
		out.WriteByte(alphabet[(n>>18)&63])
		out.WriteByte(alphabet[(n>>12)&63])
		out.WriteByte(alphabet[(n>>6)&63])
		out.WriteByte(alphabet[n&63])
	}
	return out.String()
}

func onePixelPNG() []byte {
	return []byte{
		0x89, 'P', 'N', 'G', 0x0d, 0x0a, 0x1a, 0x0a,
		0, 0, 0, 0x0d, 'I', 'H', 'D', 'R',
		0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0,
		0x1f, 0x15, 0xc4, 0x89,
		0, 0, 0, 0, 'I', 'E', 'N', 'D', 0xae, 0x42, 0x60, 0x82,
	}
}
