package taskartifacts

import (
	"bytes"
	"context"
	"fmt"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"
)

// RepositoryBinding is installed by the operator on each host. Neither remote
// URLs nor credentials arrive in a handoff payload. IDs and revisions express
// the approved mapping; matching URL text is not repository authorization.
type RepositoryBinding struct {
	ID         string `json:"id"`
	Revision   string `json:"revision"`
	Repository string `json:"repository"`
	Remote     string `json:"remote"`
	RefPrefix  string `json:"refPrefix"`
	Owner      string `json:"owner"`
	Origin     string `json:"origin"`
	Export     bool   `json:"export"`
	Import     bool   `json:"import"`
	Cleanup    bool   `json:"cleanup"`
	// An explicitly approved host adapter, never read from a repository or
	// peer. Empty means no helper; unavailable credentials fail preflight.
	CredentialHelper string `json:"credentialHelper,omitempty"`
	TLSCAFile        string `json:"tlsCAFile,omitempty"`
}

func (b RepositoryBinding) Validate() error {
	if !ID.MatchString(b.ID) || !ID.MatchString(b.Origin) || b.Revision == "" || b.Owner == "" || !filepath.IsAbs(b.Repository) {
		return fmt.Errorf("repository binding setup required")
	}
	u, err := url.Parse(b.Remote)
	if err != nil || (u.Scheme != "https" && u.Scheme != "ssh") || u.Host == "" || u.RawQuery != "" || u.Fragment != "" {
		return fmt.Errorf("binding requires an explicitly approved HTTPS or SSH remote")
	}
	if u.User != nil {
		if _, hasPassword := u.User.Password(); hasPassword || u.Scheme != "ssh" {
			return fmt.Errorf("remote URL must not contain credentials")
		}
	}
	if !strings.HasPrefix(b.RefPrefix, "refs/heads/") || strings.ContainsAny(b.RefPrefix, " ~^:?*[\\") || strings.Contains(b.RefPrefix, "..") || strings.HasSuffix(b.RefPrefix, "/") {
		return fmt.Errorf("invalid approved transfer ref prefix")
	}
	return nil
}

func (b RepositoryBinding) Ref(task, direction string) (string, error) {
	if !ID.MatchString(task) || (direction != "input" && direction != "result") {
		return "", fmt.Errorf("invalid generated ref identity")
	}
	return b.RefPrefix + "/" + b.Origin + "/" + task + "/" + direction, nil
}

func (b RepositoryBinding) GitRemote(ctx context.Context, repo string, args ...string) ([]byte, error) {
	if err := b.Validate(); err != nil {
		return nil, err
	}
	if b.TLSCAFile != "" {
		if !filepath.IsAbs(b.TLSCAFile) {
			return nil, fmt.Errorf("approved CA file must be absolute")
		}
		args = append([]string{"-c", "http.sslCAInfo=" + b.TLSCAFile}, args...)
	}
	return Git(ctx, repo, b.CredentialHelper, args...)
}

type boundedOutput struct {
	bytes.Buffer
	limit int
}

func (b *boundedOutput) Write(p []byte) (int, error) {
	if len(p) > b.limit-b.Len() {
		return 0, fmt.Errorf("Git output exceeds task limit")
	}
	return b.Buffer.Write(p)
}

// Git is dedicated to host transfer, not a worker shell. Clean config prevents
// a peer's hooks, filters, transport helpers or replace refs from executing.
// A caller must validate the host binding before network operations.
func Git(ctx context.Context, cwd, helper string, args ...string) ([]byte, error) {
	ctx, cancel := context.WithTimeout(ctx, 20*time.Second)
	defer cancel()
	base := []string{"--no-replace-objects", "-c", "core.longpaths=true", "-c", "core.alternateRefsCommand=", "-c", "core.hooksPath=" + os.DevNull, "-c", "core.fsmonitor=false", "-c", "core.untrackedCache=false", "-c", "core.attributesFile=" + os.DevNull, "-c", "core.autocrlf=false", "-c", "core.eol=lf", "-c", "core.sshCommand=ssh -F none -oBatchMode=yes", "-c", "credential.helper=", "-c", "protocol.allow=never", "-c", "protocol.https.allow=always", "-c", "protocol.ssh.allow=always", "-c", "submodule.recurse=false", "-c", "fetch.recurseSubmodules=false", "-c", "fetch.fsckObjects=true", "-c", "transfer.fsckObjects=true", "-c", "gc.auto=0"}
	if helper != "" {
		base = append(base, "-c", "credential.helper="+helper)
	}
	cmd := exec.CommandContext(ctx, "git", append(base, args...)...)
	cmd.WaitDelay = time.Second
	cmd.Dir = cwd
	for _, key := range []string{"PATH", "SystemRoot", "WINDIR", "SSH_AUTH_SOCK", "HOME", "USERPROFILE", "TMPDIR", "TEMP"} {
		if v, ok := os.LookupEnv(key); ok {
			cmd.Env = append(cmd.Env, key+"="+v)
		}
	}
	cmd.Env = append(cmd.Env, "GIT_CONFIG_NOSYSTEM=1", "GIT_CONFIG_GLOBAL="+os.DevNull, "GIT_TERMINAL_PROMPT=0", "GIT_NO_LAZY_FETCH=1", "GIT_OPTIONAL_LOCKS=0", "GIT_ATTR_NOSYSTEM=1")
	out := &boundedOutput{limit: 32 << 20}
	cmd.Stdout = out
	// Git stderr can include credential-bearing URLs from host configuration.
	// Never return it to RPC callers or workers.
	cmd.Stderr = &boundedOutput{limit: 8192}
	if err := cmd.Run(); err != nil {
		operation := "operation"
		for _, arg := range args {
			if strings.Contains("|init|config|status|rev-parse|cat-file|ls-tree|fetch|push|worktree|update-ref|merge-base|ls-remote|ls-files|", "|"+arg+"|") {
				operation = arg
				break
			}
		}
		return nil, fmt.Errorf("handoff Git %s failed or timed out; check approved repository access and checkpoint", operation)
	}
	return out.Bytes(), nil
}

// CheckSource refuses execution-valued local config before status can invoke
// clean filters. It never stages, commits, stashes or modifies the user index.
func CheckConfiguration(ctx context.Context, repo string) error {
	config, err := Git(ctx, repo, "", "config", "--local", "--name-only", "--list")
	if err != nil {
		return err
	}
	for _, key := range strings.Fields(strings.ToLower(string(config))) {
		if strings.HasPrefix(key, "filter.") || strings.HasPrefix(key, "include") || strings.HasPrefix(key, "url.") || strings.HasPrefix(key, "http.") || strings.HasPrefix(key, "credential.") || strings.HasPrefix(key, "remote.") && strings.ContainsAny(key, ":/\\") || strings.HasSuffix(key, ".promisor") || key == "extensions.partialclone" || key == "extensions.worktreeconfig" || key == "core.sparsecheckout" {
			return fmt.Errorf("unsupported checkpoint configuration: %s; use the approved host credential adapter", key)
		}
	}
	attributes, err := Git(ctx, repo, "", "rev-parse", "--path-format=absolute", "--git-path", "info/attributes")
	if err != nil {
		return err
	}
	if info, err := os.Lstat(strings.TrimSpace(string(attributes))); err == nil {
		if !info.Mode().IsRegular() || info.Size() != 0 {
			return fmt.Errorf("unsupported repository info/attributes materialization policy")
		}
	} else if !os.IsNotExist(err) {
		return fmt.Errorf("repository attributes policy could not be checked")
	}
	return nil
}

func CheckSource(ctx context.Context, repo string, selectedArtifacts ...string) (string, string, error) {
	if err := CheckConfiguration(ctx, repo); err != nil {
		return "", "", err
	}
	shallow, err := Git(ctx, repo, "", "rev-parse", "--is-shallow-repository")
	if err != nil || strings.TrimSpace(string(shallow)) != "false" {
		return "", "", fmt.Errorf("unsupported shallow or incomplete source repository")
	}
	statusArgs := []string{"status", "--porcelain=v1", "--untracked-files=all"}
	if len(selectedArtifacts) > 0 {
		statusArgs = append(statusArgs, "--", ".")
		for _, name := range selectedArtifacts {
			statusArgs = append(statusArgs, ":(exclude,literal)"+name)
		}
	}
	status, err := Git(ctx, repo, "", statusArgs...)
	if err != nil {
		return "", "", err
	}
	if len(status) != 0 {
		return "", "", fmt.Errorf("checkpoint required: selected source has dirty or untracked files; commit authorized task changes explicitly")
	}
	head, err := Git(ctx, repo, "", "rev-parse", "--verify", "HEAD^{commit}")
	if err != nil {
		return "", "", err
	}
	format, err := Git(ctx, repo, "", "rev-parse", "--show-object-format")
	return strings.TrimSpace(string(head)), strings.TrimSpace(string(format)), err
}

// VerifyTree rejects unsupported materialization BEFORE worktree creation.
// V1 requires ordinary Git blobs, no attributes, gitlinks, symlinks or LFS.
func VerifyTree(ctx context.Context, repo, commit string) error {
	listing, err := Git(ctx, repo, "", "ls-tree", "-rz", "--full-tree", commit)
	if err != nil {
		return err
	}
	var total int64
	count := 0
	seen := map[string]bool{}
	for _, row := range bytes.Split(listing, []byte{0}) {
		if len(row) == 0 {
			continue
		}
		fields := bytes.SplitN(row, []byte{'\t'}, 2)
		if len(fields) != 2 {
			return fmt.Errorf("invalid Git tree")
		}
		header := strings.Fields(string(fields[0]))
		name := string(fields[1])
		if len(header) != 3 || (header[0] != "100644" && header[0] != "100755") || header[1] != "blob" {
			return fmt.Errorf("unsupported code mode or submodule: %s", name)
		}
		// Dot files in code are supported except administrative/materialization
		// inputs. Artifact names use a narrower portable subset.
		for _, part := range strings.Split(name, "/") {
			if strings.EqualFold(part, ".git") || strings.EqualFold(part, ".workspacer") || strings.EqualFold(part, ".gitattributes") || strings.EqualFold(part, ".gitmodules") {
				return fmt.Errorf("unsupported code materialization input: %s", name)
			}
		}
		parts := strings.Split(name, "/")
		for _, part := range parts {
			if part == "." || part == ".." {
				return fmt.Errorf("invalid code path")
			}
			if err := ValidName("_" + strings.TrimPrefix(part, ".")); err != nil {
				return err
			}
			if reserved.MatchString(part) {
				return fmt.Errorf("reserved code path: %s", name)
			}
		}
		key := strings.ToLower(name)
		if seen[key] {
			return fmt.Errorf("code path case collision: %s", name)
		}
		seen[key] = true
		count++
		if count > 20000 {
			return fmt.Errorf("code entry count exceeds 20000")
		}
		blob, err := Git(ctx, repo, "", "cat-file", "blob", header[2])
		if err != nil {
			return err
		}
		total += int64(len(blob))
		if total > 512<<20 {
			return fmt.Errorf("code tree exceeds 512 MiB")
		}
		if bytes.HasPrefix(blob, []byte("version https://git-lfs.github.com/spec/v1")) {
			return fmt.Errorf("unsupported unresolved LFS input: %s", name)
		}
	}
	return nil
}
