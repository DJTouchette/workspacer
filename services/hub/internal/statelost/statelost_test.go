package statelost

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestSuspected(t *testing.T) {
	tests := []struct {
		name    string
		seed    []string // entries to create in the dir; "" = do not create the dir
		missing string
		want    bool
	}{
		{
			name:    "a directory that does not exist is a first run",
			seed:    nil,
			missing: "remote-token",
			want:    false,
		},
		{
			name:    "an empty directory is a first run",
			seed:    []string{},
			missing: "remote-token",
			want:    false,
		},
		{
			name:    "the file's own presence does not count as evidence about itself",
			seed:    []string{"remote-token"},
			missing: "remote-token",
			want:    false,
		},
		{
			name:    "any other state means somebody has run here",
			seed:    []string{"config.yaml"},
			missing: "remote-token",
			want:    true,
		},
		{
			// The false alarm this package used to produce on every genuinely
			// first boot of the Fly node. deploy/fly/node/bootstrap.sh mkdir -p's
			// plugins/, library/, layouts/, sessions/ and logs/ inside
			// <config>/workspacer before the brain starts, so counting any entry
			// meant the brain reported STATE LOSS against a volume nothing had
			// ever run on. A bare mkdir is evidence that a mkdir ran.
			name:    "an EMPTY subdirectory is a bootstrap's mkdir, not evidence somebody ran here",
			seed:    []string{"sessions/"},
			missing: "config.yaml",
			want:    false,
		},
		{
			name:    "several empty subdirectories are still a first run",
			seed:    []string{"plugins/", "library/", "layouts/", "sessions/", "logs/"},
			missing: "config.yaml",
			want:    false,
		},
		{
			// The other half of the same rule: once a directory holds something,
			// a program has run here and the missing file is a loss.
			name:    "a subdirectory with something IN it is real state",
			seed:    []string{"sessions/live.json"},
			missing: "config.yaml",
			want:    true,
		},
		{
			name:    "a file beside empty subdirectories is still real state",
			seed:    []string{"plugins/", "tokens.json"},
			missing: "config.yaml",
			want:    true,
		},
		{
			// An empty FILE is not an empty directory. A zero-byte neighbour was
			// written by something, so it counts.
			name:    "a zero-byte neighbour counts, unlike an empty directory",
			seed:    []string{"tokens.json:"},
			missing: "config.yaml",
			want:    true,
		},
		{
			name:    "a truncated file beside real state is still loss",
			seed:    []string{"remote-token", "tokens.json"},
			missing: "remote-token",
			want:    true,
		},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			dir := filepath.Join(t.TempDir(), "state")
			if tc.seed != nil {
				if err := os.MkdirAll(dir, 0o755); err != nil {
					t.Fatal(err)
				}
			}
			for _, e := range tc.seed {
				// "dir/"  = an empty directory
				// "a/b"   = a file inside a directory, so that directory is NOT empty
				// "name:" = a zero-byte file
				body := []byte("x")
				if strings.HasSuffix(e, ":") {
					e, body = strings.TrimSuffix(e, ":"), nil
				}
				p := filepath.Join(dir, e)
				var err error
				if strings.HasSuffix(e, "/") {
					err = os.MkdirAll(p, 0o755)
				} else {
					if err = os.MkdirAll(filepath.Dir(p), 0o755); err == nil {
						err = os.WriteFile(p, body, 0o600)
					}
				}
				if err != nil {
					t.Fatal(err)
				}
			}
			if got := Suspected(dir, tc.missing); got != tc.want {
				t.Errorf("Suspected = %v, want %v", got, tc.want)
			}
		})
	}
}
