package main

// These operations are accepted ONLY on the brain-owned child's private pipe.
// No method is registered on the hub bus. The child journal owns the handoff;
// Go owns daemon identity, actual launches, metadata and transport ACKs.
import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"net/http"
	"net/url"
	"sort"
	"strings"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
)

func managerGrantFingerprint(id string) (string, error) {
	sessionFacadeTokenMu.Lock()
	defer sessionFacadeTokenMu.Unlock()
	records, err := authtoken.Load(authtoken.DefaultPath())
	if err != nil {
		return "", err
	}
	for _, r := range records {
		if r.Label != sessionFacadeTokenLabelPrefix+id || r.Scope != authtoken.ScopeOperator || r.Role != "manager" {
			continue
		}
		plugins := append([]string{}, r.Plugins...)
		profiles := append([]string{}, r.ProfilesAllowed...)
		sort.Strings(plugins)
		sort.Strings(profiles)
		value := struct {
			Scope    string   `json:"scope"`
			Role     string   `json:"role"`
			Plugins  []string `json:"plugins"`
			Profiles []string `json:"profilesAllowed"`
			Yolo     bool     `json:"yoloAllowed"`
		}{"operator", "manager", plugins, profiles, r.YoloAllowed}
		var buffer bytes.Buffer
		encoder := json.NewEncoder(&buffer)
		encoder.SetEscapeHTML(false)
		if err := encoder.Encode(value); err != nil {
			return "", err
		}
		sum := sha256.Sum256(bytes.TrimSuffix(buffer.Bytes(), []byte{'\n'}))
		return hex.EncodeToString(sum[:]), nil
	}
	return "", fmt.Errorf("manager operator grant unavailable")
}

type replacementLaunch struct {
	Options json.RawMessage `json:"options"`
	Grants  string          `json:"grants"`
}
type replacementMetadata struct {
	SessionID    string          `json:"sessionId"`
	Cwd          string          `json:"cwd"`
	Label        string          `json:"label"`
	Parent       string          `json:"parentSessionId"`
	Manager      bool            `json:"isWakeTarget"`
	ResultSchema json.RawMessage `json:"resultSchema"`
}

func (r *registry) replacementHostCall(ctx context.Context, method string, params json.RawMessage) (json.RawMessage, error) {
	if method == "launch.prepare" {
		if r.callHub == nil {
			return nil, fmt.Errorf("hub callback unavailable")
		}
		return r.callHub(ctx, "plugins.prepareLaunch", params)
	}
	var p struct {
		SessionID        string                `json:"sessionId"`
		SourceSessionID  string                `json:"sourceSessionId"`
		ReceiptSessionID string                `json:"receiptSessionId"`
		Source           string                `json:"source"`
		Successor        string                `json:"successor"`
		Text             string                `json:"text"`
		Signal           string                `json:"signal"`
		Launch           replacementLaunch     `json:"launch"`
		Metadata         []replacementMetadata `json:"metadata"`
	}
	if err := json.Unmarshal(params, &p); err != nil {
		return nil, err
	}
	switch method {
	case "replacement.refresh":
		body, err := r.cm.listAllSessions(ctx)
		if err != nil {
			return nil, err
		}
		var rows []json.RawMessage
		if err := json.Unmarshal(body, &rows); err != nil {
			return nil, err
		}
		if rows == nil {
			rows = []json.RawMessage{}
		}
		for i, row := range rows {
			rows[i] = enrichAndCompat(row, r.meta)
		}
		receipt := ""
		if p.ReceiptSessionID != "" {
			receipt = r.workerFinalTurn(ctx, p.ReceiptSessionID).lastAssistant
		}
		return jsonResult(map[string]any{"snapshots": rows, "receipt": receipt})
	case "replacement.send":
		if err := validateSessionConfigName(p.SessionID); err != nil {
			return nil, err
		}
		if p.Text == "" || len(p.Text) > 256*1024 {
			return nil, fmt.Errorf("invalid handoff message")
		}
		return jsonResult(map[string]string{"status": r.cm.deliverCaptured(ctx, p.SessionID, p.Text)})
	case "replacement.signal", "replacement.close":
		if err := validateSessionConfigName(p.SessionID); err != nil {
			return nil, err
		}
		signal := p.Signal
		if method == "replacement.close" {
			signal = "SIGTERM"
		}
		if signal != "SIGINT" && signal != "SIGTERM" {
			return nil, fmt.Errorf("invalid handoff signal")
		}
		if body, err := r.cm.getSession(ctx, p.SessionID); err == nil {
			var row struct {
				Mode string `json:"mode"`
			}
			_ = json.Unmarshal(body, &row)
			if row.Mode != "stopped" {
				if err := r.cm.signal(ctx, p.SessionID, signal); err != nil {
					return nil, err
				}
			}
		} else {
			return nil, err
		}
		if method == "replacement.close" {
			if err := revokeSessionFacadeToken(p.SessionID); err != nil {
				return nil, err
			}
		}
		return okResult()
	case "replacement.spawn":
		if err := validateSessionConfigName(p.SessionID); err != nil {
			return nil, err
		}
		fingerprint, err := managerGrantFingerprint(p.SourceSessionID)
		if err != nil || fingerprint != p.Launch.Grants {
			return nil, fmt.Errorf("source manager grants changed")
		}
		var options spawnParams
		if err := json.Unmarshal(p.Launch.Options, &options); err != nil {
			return nil, err
		}
		body, err := r.cm.getSession(ctx, p.SourceSessionID)
		if err != nil {
			return nil, err
		}
		var source struct {
			Cwd      string `json:"cwd"`
			Provider string `json:"provider"`
			Mode     string `json:"mode"`
		}
		if err := json.Unmarshal(body, &source); err != nil {
			return nil, err
		}
		if source.Mode == "stopped" || source.Cwd != options.Cwd || source.Provider != options.Provider || !options.Manager || options.ToolScope != "operator" {
			return nil, fmt.Errorf("source manager identity changed")
		}
		request, err := http.NewRequestWithContext(ctx, http.MethodGet, r.cm.base+"/sessions/"+url.PathEscape(p.SessionID)+"?summary_meta=1", nil)
		if err != nil {
			return nil, err
		}
		response, err := r.cm.http.Do(request)
		if err != nil {
			return nil, err
		}
		response.Body.Close()
		if response.StatusCode != http.StatusNotFound {
			return nil, fmt.Errorf("successor absence was not confirmed (HTTP %d); inspect before retrying", response.StatusCode)
		}
		var wire map[string]any
		if err := json.Unmarshal(p.Launch.Options, &wire); err != nil {
			return nil, err
		}
		delete(wire, "resumeSessionId")
		delete(wire, "message")
		delete(wire, "firstMessage")
		delete(wire, "launchIntegrationId")
		delete(wire, "launchIntegrationGranted")
		wire["transport"] = "stream"
		wire["exactModel"] = true
		wire["yoloGranted"] = true
		wire["profileGranted"] = true
		raw, _ := json.Marshal(wire)
		return r.spawnCore(ctx, raw, desktopSpawnMetadata{ReplacementID: p.SessionID})
	case "replacement.restore", "replacement.transfer":
		if r.meta == nil {
			return nil, fmt.Errorf("spawn metadata unavailable")
		}
		if len(p.Metadata) > 2048 {
			return nil, fmt.Errorf("too much replacement metadata")
		}
		if method == "replacement.transfer" {
			fingerprint, err := managerGrantFingerprint(p.Successor)
			if err != nil || fingerprint != p.Launch.Grants {
				return nil, fmt.Errorf("successor grants changed before transfer")
			}
			body, err := r.cm.getSession(ctx, p.Successor)
			if err != nil {
				return nil, err
			}
			var row struct {
				Mode     string `json:"mode"`
				Cwd      string `json:"cwd"`
				Provider string `json:"provider"`
			}
			_ = json.Unmarshal(body, &row)
			var launch spawnParams
			_ = json.Unmarshal(p.Launch.Options, &launch)
			if row.Mode == "stopped" || row.Cwd != launch.Cwd || row.Provider != launch.Provider {
				return nil, fmt.Errorf("successor changed before transfer")
			}
		}
		ids := []string{}
		for _, m := range p.Metadata {
			if err := validateSessionConfigName(m.SessionID); err != nil {
				return nil, err
			}
			meta, _ := r.meta.get(m.SessionID)
			meta.Label = m.Label
			meta.ParentSessionID = m.Parent
			meta.IsWakeTarget = m.Manager
			meta.ResultSchema = m.ResultSchema
			if method == "replacement.transfer" && meta.ParentSessionID == p.Source {
				meta.ParentSessionID = p.Successor
			}
			r.meta.set(m.SessionID, meta)
			ids = append(ids, m.SessionID)
		}
		if method == "replacement.transfer" {
			r.watchMu.Lock()
			for _, watch := range r.watches {
				if watch.WatcherSessionID == p.Source {
					watch.WatcherSessionID = p.Successor
				}
			}
			r.watchMu.Unlock()
		}
		r.restampParents(ids)
		return okResult()
	default:
		return nil, fmt.Errorf("unknown private lifecycle operation %q", strings.TrimSpace(method))
	}
}

func revokeSessionFacadeToken(id string) error {
	sessionFacadeTokenMu.Lock()
	defer sessionFacadeTokenMu.Unlock()
	file := authtoken.DefaultPath()
	rows, err := authtoken.Load(file)
	if err != nil {
		return err
	}
	kept := make([]authtoken.Record, 0, len(rows))
	for _, row := range rows {
		if row.Label != sessionFacadeTokenLabelPrefix+id {
			kept = append(kept, row)
		}
	}
	return authtoken.Save(file, kept)
}
