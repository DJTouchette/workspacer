package integration

import (
	"context"
	"encoding/json"
	"net/http/httptest"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/coder/websocket"
	"github.com/djtouchette/workspacer-hub/internal/broker"
	"github.com/djtouchette/workspacer-hub/internal/bus"
	"github.com/djtouchette/workspacer-hub/internal/plugin"
)

func dialBus(t *testing.T, url string) *websocket.Conn {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()
	c, _, err := websocket.Dial(ctx, url, nil)
	if err != nil {
		t.Fatalf("dial %s: %v", url, err)
	}
	return c
}

func jsonParams(t *testing.T, m map[string]any) json.RawMessage {
	t.Helper()
	b, err := json.Marshal(m)
	if err != nil {
		t.Fatal(err)
	}
	return b
}

// TestEditorPaneTokenHasAmbientFilesystemAccess proves legacy pane path bindings
// are inert: the real editor manifest may call fs.read both inside and outside
// the agent cwd once the plugin is enabled.
func TestEditorPaneTokenHasAmbientFilesystemAccess(t *testing.T) {
	b := broker.New()
	srv := bus.NewServer(b)
	srv.SetToken("host") // distinguishes the trusted provider from the plugin token

	busHTTP := httptest.NewServer(srv.Handler())
	defer busHTTP.Close()
	wsURL := strings.Replace(busHTTP.URL, "http://", "ws://", 1) + "/bus"

	// Manager wired to the real bus as token registrar; load + add the *actual*
	// editor plugin, then mint a pane token bound to a project directory — exactly
	// what the desktop host does when it opens the editor on an agent.
	mgr := plugin.NewManager(b, srv)
	mf, err := plugin.Load(filepath.Join("..", "..", "examples", "editor", "plugin.json"))
	if err != nil {
		t.Fatalf("load editor manifest: %v", err)
	}
	// Point the plugin dir at a temp dir so Add's token persistence doesn't write
	// a .bus-token into the source tree. The editor's fs scope is ${agentCwd},
	// not ${pluginDir}, so this doesn't affect what we're testing.
	mf.Dir = t.TempDir()
	mgr.Add(mf)

	project := t.TempDir()
	paneTok, err := mgr.PaneToken("workspacer.editor", map[string]string{"agentCwd": project})
	if err != nil {
		t.Fatalf("PaneToken: %v", err)
	}

	// Trusted provider answers fs.read and records an out-of-project call.
	prov := dialBus(t, wsURL+"?token=host")
	defer prov.CloseNow()
	readUntil(t, prov, "hello")
	send(t, prov, bus.Frame{Op: "register", Methods: []string{"fs.read"}})
	readUntil(t, prov, "registered")
	sawOutside := make(chan string, 1)
	go func() {
		for {
			ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
			_, data, err := prov.Read(ctx)
			cancel()
			if err != nil {
				return
			}
			var f bus.Frame
			if json.Unmarshal(data, &f) != nil || f.Op != "call" {
				continue
			}
			// Flag if a path outside the project ever reached the provider.
			var p struct {
				Path string `json:"path"`
			}
			_ = json.Unmarshal(f.Params, &p)
			if !strings.HasPrefix(p.Path, project) {
				select {
				case sawOutside <- p.Path:
				default:
				}
			}
			out, _ := json.Marshal(bus.Frame{Op: "result", ID: f.ID, Result: json.RawMessage(`{"contents":"hi","size":2}`)})
			wctx, wcancel := context.WithTimeout(context.Background(), 2*time.Second)
			_ = prov.Write(wctx, websocket.MessageText, out)
			wcancel()
		}
	}()

	// The editor's webview connects with its per-pane token.
	cl := dialBus(t, wsURL+"?token="+paneTok)
	defer cl.CloseNow()
	readUntil(t, cl, "hello")

	// A read inside the project → routed, returns the provider's result.
	send(t, cl, bus.Frame{Op: "call", ID: "in", Method: "fs.read",
		Params: jsonParams(t, map[string]any{"path": filepath.Join(project, "main.go")})})
	if r := readUntil(t, cl, "result"); r.ID != "in" {
		t.Fatalf("in-scope read: got id %q, want in", r.ID)
	}

	// A read outside the project is also routed: Workspacer no longer applies a
	// filesystem-root grant to enabled plugins.
	send(t, cl, bus.Frame{Op: "call", ID: "out", Method: "fs.read",
		Params: jsonParams(t, map[string]any{"path": "/etc/passwd"})})
	if r := readUntil(t, cl, "result"); r.ID != "out" {
		t.Fatalf("ambient read: got id %q, want out", r.ID)
	}

	select {
	case leaked := <-sawOutside:
		if leaked != "/etc/passwd" {
			t.Fatalf("provider saw %q, want /etc/passwd", leaked)
		}
	case <-time.After(time.Second):
		t.Fatal("ambient absolute path never reached the provider")
	}
}
