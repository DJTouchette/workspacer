package plugin

import (
	"archive/tar"
	"compress/gzip"
	"context"
	"fmt"
	"io"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"time"
)

// expandPlatformTokens substitutes ${os}, ${arch}, and ${exe} with the host's
// GOOS, GOARCH, and the executable suffix (".exe" on Windows, "" elsewhere), so
// a manifest can point at a prebuilt per-platform binary without per-OS forks,
// e.g. server.command "./bin/${os}-${arch}/server${exe}".
func expandPlatformTokens(s string) string {
	exe := ""
	if runtime.GOOS == "windows" {
		exe = ".exe"
	}
	return strings.NewReplacer(
		"${os}", runtime.GOOS,
		"${arch}", runtime.GOARCH,
		"${exe}", exe,
	).Replace(s)
}

// expandPlatformTokensAll expands platform tokens in each element of ss.
func expandPlatformTokensAll(ss []string) []string {
	if len(ss) == 0 {
		return ss
	}
	out := make([]string, len(ss))
	for i, s := range ss {
		out[i] = expandPlatformTokens(s)
	}
	return out
}

// Inspect downloads a plugin (GitHub URL / owner-repo / tarball) and returns its
// manifest WITHOUT installing or running anything, so the UI can show what the
// plugin is and what it requires before the user commits. The download is
// extracted to a throwaway temp dir that's removed before returning.
func Inspect(input string) (Manifest, error) {
	urls, _, err := resolveTarballURLs(input)
	if err != nil {
		return Manifest{}, err
	}
	var lastErr error
	for _, url := range urls {
		m, err := inspectTarball(url)
		if err == nil {
			return m, nil
		}
		lastErr = err
	}
	return Manifest{}, lastErr
}

func inspectTarball(tarballURL string) (Manifest, error) {
	ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
	defer cancel()
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, tarballURL, nil)
	if err != nil {
		return Manifest{}, err
	}
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return Manifest{}, err
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return Manifest{}, fmt.Errorf("download %s: HTTP %d", tarballURL, resp.StatusCode)
	}
	tmp, err := os.MkdirTemp("", "wks-inspect-")
	if err != nil {
		return Manifest{}, err
	}
	defer os.RemoveAll(tmp)
	if err := extractTarGz(resp.Body, tmp, 1); err != nil {
		return Manifest{}, fmt.Errorf("extract: %w", err)
	}
	src, err := locateManifestDir(tmp)
	if err != nil {
		return Manifest{}, err
	}
	// Dir points into the temp we're about to delete, but the UI only reads the
	// declarative fields (id/name/server/install/panes), not Dir.
	return Load(filepath.Join(src, "plugin.json"))
}

// Install fetches a plugin from a GitHub URL (or a direct .tar.gz URL),
// extracts it into pluginsDir, runs its optional one-time install command, and
// returns the validated manifest. The caller adds it to the Manager.
//
// NOTE: this downloads and (via the Manager) RUNS code from the internet — the
// caller is responsible for getting user consent. It's the trusted-install
// model (like a VS Code extension), not a sandbox.
func Install(pluginsDir, input string, progress func(stage string)) (Manifest, error) {
	if progress == nil {
		progress = func(string) {}
	}
	if pluginsDir == "" {
		return Manifest{}, fmt.Errorf("no plugins directory configured")
	}
	if err := os.MkdirAll(pluginsDir, 0o755); err != nil {
		return Manifest{}, err
	}

	urls, name, err := resolveTarballURLs(input)
	if err != nil {
		return Manifest{}, err
	}

	var lastErr error
	for _, url := range urls {
		m, err := installFromTarball(pluginsDir, url, name, progress)
		if err == nil {
			// Record the install reference next to the plugin so the UI can offer
			// one-click update. Best-effort: a missing source just disables update.
			if writeErr := os.WriteFile(filepath.Join(m.Dir, sourceFile), []byte(input), 0o644); writeErr != nil {
				return m, nil
			}
			m.Source = input
			return m, nil
		}
		lastErr = err
	}
	return Manifest{}, lastErr
}

// InstallFromDir installs a plugin from a local source directory — used to add
// one of the bundled examples that ship inside the app. It validates the source
// manifest, copies the tree into pluginsDir, runs the optional install command,
// and returns the validated manifest. The caller adds it to the Manager.
//
// Unlike Install it neither downloads nor writes an .install-source: a bundled
// example has nothing to "update" from, so the UI shows no Update button.
func InstallFromDir(pluginsDir, srcDir string) (Manifest, error) {
	if pluginsDir == "" {
		return Manifest{}, fmt.Errorf("no plugins directory configured")
	}
	if err := os.MkdirAll(pluginsDir, 0o755); err != nil {
		return Manifest{}, err
	}
	// Validate before copying anything.
	m, err := Load(filepath.Join(srcDir, "plugin.json"))
	if err != nil {
		return Manifest{}, err
	}
	name := sanitizeName(m.ID)
	if name == "" {
		return Manifest{}, fmt.Errorf("could not determine install directory name")
	}
	dest := filepath.Join(pluginsDir, name)
	if err := os.RemoveAll(dest); err != nil { // overwrite on re-add
		return Manifest{}, err
	}
	if err := os.CopyFS(dest, os.DirFS(srcDir)); err != nil {
		return Manifest{}, fmt.Errorf("copy example: %w", err)
	}
	final, err := Load(filepath.Join(dest, "plugin.json"))
	if err != nil {
		return Manifest{}, err
	}
	if len(final.Install) > 0 {
		if err := runInstall(dest, final.Install); err != nil {
			return Manifest{}, fmt.Errorf("install command failed: %w", err)
		}
	}
	return final, nil
}

// resolveTarballURLs turns a GitHub reference into candidate codeload tarball
// URLs (trying the given ref, else main then master), plus a fallback dir name.
// A direct .tar.gz URL is returned as-is.
func resolveTarballURLs(input string) (urls []string, name string, err error) {
	input = strings.TrimSpace(input)
	if input == "" {
		return nil, "", fmt.Errorf("empty plugin URL")
	}
	if strings.HasSuffix(input, ".tar.gz") || strings.HasSuffix(input, ".tgz") {
		return []string{input}, "", nil
	}

	s := input
	for _, p := range []string{"https://", "http://", "git@"} {
		s = strings.TrimPrefix(s, p)
	}
	s = strings.TrimPrefix(s, "github.com/")
	s = strings.TrimPrefix(s, "github.com:")
	s = strings.TrimSuffix(s, ".git")
	s = strings.Trim(s, "/")

	parts := strings.Split(s, "/")
	if len(parts) < 2 || parts[0] == "" || parts[1] == "" {
		return nil, "", fmt.Errorf("not a github repo: %q (want owner/repo or a github URL)", input)
	}
	owner, repo := parts[0], parts[1]
	name = repo

	var refs []string
	if len(parts) >= 4 && (parts[2] == "tree" || parts[2] == "commit") {
		refs = []string{parts[3]}
	} else {
		refs = []string{"main", "master"}
	}
	for _, ref := range refs {
		urls = append(urls, fmt.Sprintf("https://codeload.github.com/%s/%s/tar.gz/%s", owner, repo, ref))
	}
	return urls, name, nil
}

func installFromTarball(pluginsDir, tarballURL, fallbackName string, progress func(stage string)) (Manifest, error) {
	if progress == nil {
		progress = func(string) {}
	}
	ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
	defer cancel()
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, tarballURL, nil)
	if err != nil {
		return Manifest{}, err
	}
	progress("downloading")
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return Manifest{}, err
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return Manifest{}, fmt.Errorf("download %s: HTTP %d", tarballURL, resp.StatusCode)
	}

	// Extract into a temp dir inside pluginsDir (same filesystem → cheap rename).
	tmp, err := os.MkdirTemp(pluginsDir, ".install-")
	if err != nil {
		return Manifest{}, err
	}
	defer os.RemoveAll(tmp)

	// GitHub tarballs wrap everything in a single top-level "<repo>-<ref>/" dir;
	// strip it so the plugin's files land at the temp root.
	progress("extracting")
	if err := extractTarGz(resp.Body, tmp, 1); err != nil {
		return Manifest{}, fmt.Errorf("extract: %w", err)
	}

	// Locate the dir holding plugin.json: the temp root, or one level down.
	src, err := locateManifestDir(tmp)
	if err != nil {
		return Manifest{}, err
	}
	m, err := Load(filepath.Join(src, "plugin.json"))
	if err != nil {
		return Manifest{}, err
	}

	name := sanitizeName(m.ID)
	if name == "" {
		name = sanitizeName(fallbackName)
	}
	if name == "" {
		return Manifest{}, fmt.Errorf("could not determine install directory name")
	}
	dest := filepath.Join(pluginsDir, name)
	if err := os.RemoveAll(dest); err != nil { // reinstall / update
		return Manifest{}, err
	}
	if err := os.Rename(src, dest); err != nil {
		return Manifest{}, err
	}

	// Re-load from the final location so Dir is correct.
	final, err := Load(filepath.Join(dest, "plugin.json"))
	if err != nil {
		return Manifest{}, err
	}

	if len(final.Install) > 0 {
		progress("building")
		if err := runInstall(dest, final.Install); err != nil {
			return Manifest{}, fmt.Errorf("install command failed: %w", err)
		}
	}
	return final, nil
}

// runInstall runs a plugin's declared install/build command.
//
// SECURITY / SCOPE: this executes arbitrary code from the plugin UNCONFINED —
// full user privileges, no sandbox, only a 5-minute wall-clock timeout. That is
// intentional for the trusted-install model (the caller gets user consent, like
// a VS Code extension), but it means a plugin's build step can do anything the
// user can. Sandboxing this (namespaces / seccomp / a restricted PATH) is a
// larger design deliberately out of scope for the extraction-bounds pass; the
// download+extract path above is now size-bounded, this exec is not.
func runInstall(dir string, argv []string) error {
	argv = expandPlatformTokensAll(argv)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
	defer cancel()
	cmd := exec.CommandContext(ctx, argv[0], argv[1:]...)
	cmd.Dir = dir
	out, err := cmd.CombinedOutput()
	if err != nil {
		return fmt.Errorf("%v: %s", err, strings.TrimSpace(string(out)))
	}
	return nil
}

// locateManifestDir returns the dir under root that contains plugin.json
// (root itself, or a single immediate subdirectory).
func locateManifestDir(root string) (string, error) {
	if _, err := os.Stat(filepath.Join(root, "plugin.json")); err == nil {
		return root, nil
	}
	entries, err := os.ReadDir(root)
	if err != nil {
		return "", err
	}
	for _, e := range entries {
		if e.IsDir() {
			if _, err := os.Stat(filepath.Join(root, e.Name(), "plugin.json")); err == nil {
				return filepath.Join(root, e.Name()), nil
			}
		}
	}
	return "", fmt.Errorf("no plugin.json found in the downloaded archive")
}

func sanitizeName(s string) string {
	s = strings.ToLower(strings.TrimSpace(s))
	var b strings.Builder
	for _, r := range s {
		switch {
		case r >= 'a' && r <= 'z', r >= '0' && r <= '9':
			b.WriteRune(r)
		case r == '.' || r == '_' || r == '/' || r == ' ':
			b.WriteRune('-')
		}
	}
	return strings.Trim(b.String(), "-")
}

// Extraction bounds. A gzipped tar can expand to far more than it downloads (a
// decompression bomb), so streaming extraction is capped independently of the
// 60s download timeout: total uncompressed bytes, file count, and any single
// file's size. Generous enough for real plugins, small enough that a hostile
// archive fails fast instead of filling the disk.
const (
	maxExtractedBytes = 512 << 20 // 512 MiB total, across all files
	maxExtractedFiles = 10_000    // entry count (files + dirs)
	maxSingleFileSize = 128 << 20 // 128 MiB for any one file
)

// extractLimits bounds a single extraction. Split out from the constants so
// tests can drive the same guards with tiny limits instead of building
// gigabyte archives.
type extractLimits struct {
	maxBytes    int64 // total uncompressed bytes across all files
	maxFiles    int   // entry count (files + dirs)
	maxFileSize int64 // any single file
}

var defaultExtractLimits = extractLimits{
	maxBytes:    maxExtractedBytes,
	maxFiles:    maxExtractedFiles,
	maxFileSize: maxSingleFileSize,
}

// extractTarGz extracts a gzipped tar into dest, stripping the first
// stripComponents path segments, guarding against path traversal (zip-slip) and
// decompression bombs (the production bounds in defaultExtractLimits).
func extractTarGz(r io.Reader, dest string, stripComponents int) error {
	return extractTarGzLimited(r, dest, stripComponents, defaultExtractLimits)
}

func extractTarGzLimited(r io.Reader, dest string, stripComponents int, lim extractLimits) error {
	gz, err := gzip.NewReader(r)
	if err != nil {
		return err
	}
	defer gz.Close()
	tr := tar.NewReader(gz)
	destAbs, err := filepath.Abs(dest)
	if err != nil {
		return err
	}
	var totalBytes int64
	var entries int
	for {
		hdr, err := tr.Next()
		if err == io.EOF {
			return nil
		}
		if err != nil {
			return err
		}
		if entries++; entries > lim.maxFiles {
			return fmt.Errorf("archive has too many entries (limit %d): possible decompression bomb", lim.maxFiles)
		}
		name := stripPath(hdr.Name, stripComponents)
		if name == "" {
			continue
		}
		target := filepath.Join(destAbs, name)
		// zip-slip guard: target must stay within destAbs.
		if rel, err := filepath.Rel(destAbs, target); err != nil || rel == ".." || strings.HasPrefix(rel, ".."+string(filepath.Separator)) {
			return fmt.Errorf("unsafe path in archive: %q", hdr.Name)
		}
		switch hdr.Typeflag {
		case tar.TypeDir:
			if err := os.MkdirAll(target, 0o755); err != nil {
				return err
			}
		case tar.TypeReg:
			if err := os.MkdirAll(filepath.Dir(target), 0o755); err != nil {
				return err
			}
			f, err := os.OpenFile(target, os.O_CREATE|os.O_TRUNC|os.O_WRONLY, 0o644)
			if err != nil {
				return err
			}
			// Cap this file, and the running total, without trusting hdr.Size
			// (a lying header must not let us over-read). LimitReader stops one
			// byte past the smaller of the two remaining budgets so we can tell
			// which bound was hit.
			fileBudget := lim.maxFileSize
			if remaining := lim.maxBytes - totalBytes; remaining < fileBudget {
				fileBudget = remaining
			}
			n, err := io.Copy(f, io.LimitReader(tr, fileBudget+1))
			f.Close()
			if err != nil {
				return err
			}
			totalBytes += n
			if n > lim.maxFileSize {
				return fmt.Errorf("file %q exceeds the per-file limit (%d bytes): possible decompression bomb", name, lim.maxFileSize)
			}
			if totalBytes > lim.maxBytes {
				return fmt.Errorf("archive exceeds the total extraction limit (%d bytes): possible decompression bomb", lim.maxBytes)
			}
		}
	}
}

func stripPath(name string, n int) string {
	name = filepath.Clean("/" + name)[1:] // normalize, drop leading slash
	parts := strings.Split(name, string(filepath.Separator))
	if len(parts) <= n {
		return ""
	}
	return filepath.Join(parts[n:]...)
}
