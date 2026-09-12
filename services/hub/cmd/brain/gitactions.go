package main

// Headless twins of gitService.ts's review actions. Run from the derived repo
// root, but bind filesystem operands to the canonical path the guard checked.
import (
	"context"
	"encoding/json"
	"fmt"
	"path/filepath"
	"regexp"
	"strings"
)

var commitHashPattern = regexp.MustCompile(`(?i)^[0-9a-f]{4,40}$`)

func (r *registry) gitCommitDiffCall(ctx context.Context, raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		Cwd  string `json:"cwd"`
		Hash string `json:"hash"`
		Path string `json:"path"`
	}
	if err := json.Unmarshal(raw, &p); err != nil {
		return nil, err
	}
	cwd, err := r.guardGitCwd(ctx, "git.commitDiff", p.Cwd)
	if err != nil {
		return nil, err
	}
	root, err := gitRootOrErr(ctx, cwd)
	if err != nil {
		return nil, err
	}
	hash := strings.TrimSpace(p.Hash)
	if !commitHashPattern.MatchString(hash) {
		return nil, fmt.Errorf("not a commit hash: %s", p.Hash)
	}
	args := []string{"show", "--format=", "--patch", hash}
	if p.Path != "" {
		operand, err := r.anchorGitPathspec(ctx, "git.commitDiff", cwd, p.Path, nil)
		if err != nil {
			return nil, err
		}
		args = append(args, "--", operand)
	}
	result, err := runGit(ctx, root, args)
	if err != nil {
		return nil, err
	}
	if !result.ok {
		return nil, gitReadError(result, "git show failed")
	}
	return json.Marshal(map[string]any{"diff": result.stdout})
}

func (r *registry) gitCommitNumstatCall(ctx context.Context, raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		Cwd  string `json:"cwd"`
		Hash string `json:"hash"`
	}
	if err := json.Unmarshal(raw, &p); err != nil {
		return nil, err
	}
	cwd, err := r.guardGitCwd(ctx, "git.commitNumstat", p.Cwd)
	if err != nil {
		return nil, err
	}
	root, err := gitRootOrErr(ctx, cwd)
	if err != nil {
		return nil, err
	}
	hash := strings.TrimSpace(p.Hash)
	if !commitHashPattern.MatchString(hash) {
		return nil, fmt.Errorf("not a commit hash: %s", p.Hash)
	}
	result, err := runGit(ctx, root, []string{"show", "--format=", "--numstat", hash})
	if err != nil {
		return nil, err
	}
	if !result.ok {
		return nil, gitReadError(result, "git show failed")
	}
	return json.Marshal(map[string]any{"files": parseGitNumstat(result.stdout)})
}

// Pathless stage/unstage means the allowed cwd, never an unvalidated ancestor
// repository. This is the desktop bus contract's cwdPathspec rule.
func (r *registry) gitMutationOperand(ctx context.Context, method, cwd, target string) (string, error) {
	if target != "" {
		return r.anchorGitPathspec(ctx, method, cwd, target, [][]string{r.workspaceRoots(ctx)})
	}
	root, err := gitRootOrErr(ctx, cwd)
	if err != nil {
		return "", err
	}
	checked, err := assertPathAllowed(method, cwd, []string{root})
	if err != nil {
		return "", err
	}
	canonical, ok := canonicalRoot(root)
	if !ok {
		return "", fmt.Errorf("%s: cannot resolve repository root", method)
	}
	return filepath.Rel(canonical, checked)
}

func (r *registry) gitStageCall(ctx context.Context, raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		Cwd  string `json:"cwd"`
		Path string `json:"path"`
	}
	if err := json.Unmarshal(raw, &p); err != nil {
		return nil, err
	}
	cwd, err := r.guardGitCwd(ctx, "git.stage", p.Cwd)
	if err != nil {
		return nil, err
	}
	operand, err := r.gitMutationOperand(ctx, "git.stage", cwd, p.Path)
	if err != nil {
		return nil, err
	}
	return gitAction(ctx, cwd, []string{"add", "-A", "--", operand})
}

func (r *registry) gitUnstageCall(ctx context.Context, raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		Cwd  string `json:"cwd"`
		Path string `json:"path"`
	}
	if err := json.Unmarshal(raw, &p); err != nil {
		return nil, err
	}
	cwd, err := r.guardGitCwd(ctx, "git.unstage", p.Cwd)
	if err != nil {
		return nil, err
	}
	operand, err := r.gitMutationOperand(ctx, "git.unstage", cwd, p.Path)
	if err != nil {
		return nil, err
	}
	return gitAction(ctx, cwd, []string{"reset", "-q", "HEAD", "--", operand})
}

func (r *registry) gitCommitCall(ctx context.Context, raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		Cwd     string `json:"cwd"`
		Message string `json:"message"`
	}
	if err := json.Unmarshal(raw, &p); err != nil {
		return nil, err
	}
	cwd, err := r.guardGitCwd(ctx, "git.commit", p.Cwd)
	if err != nil {
		return nil, err
	}
	if strings.TrimSpace(p.Message) == "" {
		return nil, fmt.Errorf("empty commit message")
	}
	return gitAction(ctx, cwd, []string{"commit", "-m", p.Message})
}

func (r *registry) gitPushCall(ctx context.Context, raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		Cwd string `json:"cwd"`
	}
	if err := json.Unmarshal(raw, &p); err != nil {
		return nil, err
	}
	cwd, err := r.guardGitCwd(ctx, "git.push", p.Cwd)
	if err != nil {
		return nil, err
	}
	return gitAction(ctx, cwd, []string{"push"})
}

func gitAction(ctx context.Context, cwd string, args []string) (json.RawMessage, error) {
	root, err := gitRootOrErr(ctx, cwd)
	if err != nil {
		return nil, err
	}
	result, err := runGit(ctx, root, args)
	if err != nil {
		return nil, err
	}
	if !result.ok {
		return nil, fmt.Errorf("%s", formatGitActionError(strings.TrimSpace(result.stderr+"\n"+result.stdout), "git "+args[0]+" failed"))
	}
	return json.Marshal(map[string]any{"ok": true, "output": result.stdout})
}

func formatGitActionError(raw, fallback string) string {
	message := strings.TrimSpace(raw)
	if message == "" {
		message = fallback
	}
	lower := strings.ToLower(message)
	for _, row := range []struct {
		needles []string
		summary string
	}{
		{[]string{"you have unmerged paths", "fix conflicts", "conflict (", "merge conflict"}, "Merge conflicts need resolution before this git action can continue. Resolve the conflicted files, stage them, then retry."},
		{[]string{"no changes added to commit", "nothing to commit"}, "Nothing is staged to commit. Stage files in Review, then commit again."},
		{[]string{"no upstream branch"}, "No upstream branch is configured. Set an upstream with git push --set-upstream, then retry."},
		{[]string{"non-fast-forward", "fetch first", "updates were rejected", "rejected"}, "Push was rejected because the remote has changes this branch does not have. Pull or rebase, resolve anything needed, then retry."},
	} {
		for _, needle := range row.needles {
			if strings.Contains(lower, needle) {
				return row.summary + "\n\n" + message
			}
		}
	}
	return message
}
