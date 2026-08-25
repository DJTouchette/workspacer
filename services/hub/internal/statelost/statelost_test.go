package statelost

import (
	"os"
	"path/filepath"
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
			name:    "a subdirectory counts too",
			seed:    []string{"sessions/"},
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
				p := filepath.Join(dir, e)
				var err error
				if e[len(e)-1] == '/' {
					err = os.MkdirAll(p, 0o755)
				} else {
					err = os.WriteFile(p, []byte("x"), 0o600)
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
