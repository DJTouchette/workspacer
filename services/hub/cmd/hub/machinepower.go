package main

// Power of THIS host, independent of the remote worker registry. A stopped
// host cannot serve a wake RPC; wake belongs to an external provider/proxy.
import (
	"context"
	"encoding/json"
	"fmt"
	"log"
	"net/url"
	"os"
	"strings"
	"sync"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/bus"
	"github.com/djtouchette/workspacer-hub/internal/flyapi"
	"github.com/djtouchette/workspacer-hub/internal/nodes"
	"github.com/djtouchette/workspacer-hub/internal/quiescence"
)

type machinePowerProvider interface {
	Check(context.Context) error
	Stop(context.Context) error
}

type flyMachinePower struct {
	client  flyapi.Client
	app, id string
}

func (p flyMachinePower) Check(ctx context.Context) error {
	_, err := p.client.State(ctx, p.app, p.id)
	return err
}
func (p flyMachinePower) Stop(ctx context.Context) error {
	return p.client.Stop(ctx, p.app, p.id, "SIGTERM", 45*time.Second)
}

type machinePower struct {
	provider    machinePowerProvider
	wakeURL     string
	label       string
	disconnect  func()
	mu          sync.Mutex
	stopping    bool
	manualStop  bool
	lastError   string
	observeOnly bool
	idleTimeout time.Duration
	idleState   quiescence.Result
}

// Explicit opt-in: never infer permission to stop a user's laptop from its OS
// or expose Stop on a server with no configured external wake path.
func configuredMachinePower(disconnect func()) *machinePower {
	p := &machinePower{disconnect: disconnect, idleTimeout: machineIdleTimeout(), observeOnly: os.Getenv("WKS_MACHINE_IDLE_MODE") != "stop"}
	if os.Getenv("WKS_MACHINE_POWER") != "fly" || os.Getenv("WKS_MACHINE_WAKE") != "http" {
		return p
	}
	app, id := strings.TrimSpace(os.Getenv("FLY_APP_NAME")), strings.TrimSpace(os.Getenv("FLY_MACHINE_ID"))
	token, err := nodes.ResolveToken(&nodes.Fly{Token: strings.TrimSpace(os.Getenv("FLY_API_TOKEN")), TokenFile: os.Getenv("FLY_API_TOKEN_FILE")})
	if err != nil {
		log.Print("machine power: could not read power credential file")
		return p
	}
	if raw := os.Getenv("WKS_MACHINE_WAKE_URL"); raw != "" {
		u, err := url.Parse(raw)
		if err != nil || u.Scheme != "https" || u.Host == "" || u.User != nil || u.RawQuery != "" || u.Fragment != "" {
			log.Print("machine power: wake URL must be a credential-free HTTPS URL")
			return p
		}
		p.wakeURL = u.String()
	}
	if app == "" || id == "" || token == "" {
		log.Print("machine power: Fly requires FLY_APP_NAME, FLY_MACHINE_ID and FLY_API_TOKEN; Stop is disabled")
		return p
	}
	p.provider = flyMachinePower{client: flyapi.New(token), app: app, id: id}
	p.label = app
	return p
}

func machinePowerTrusted(method string, c bus.CallerIdentity) error {
	if !c.IsTrusted() || c.Scope != "operator" {
		return fmt.Errorf("%s requires operator authority", method)
	}
	return nil
}

func machinePowerInfo(p *machinePower) bus.LocalIdentHandler {
	return func(c bus.CallerIdentity, _ json.RawMessage) (any, error) {
		p.mu.Lock()
		defer p.mu.Unlock()
		return map[string]any{"canStop": p.provider != nil && machinePowerTrusted("machine.power", c) == nil,
			"wake": "http", "wakeUrl": p.wakeURL, "label": p.label, "stopping": p.stopping, "error": p.lastError,
			"idleTimeoutSeconds": int64(p.idleTimeout / time.Second), "idleMode": p.idleMode(), "idle": p.idleState}, nil
	}
}

func machineStop(p *machinePower) bus.LocalIdentHandler {
	return func(c bus.CallerIdentity, _ json.RawMessage) (any, error) {
		if err := machinePowerTrusted("machine.stop", c); err != nil {
			return nil, err
		}
		p.mu.Lock()
		defer p.mu.Unlock()
		if p.provider == nil {
			return nil, fmt.Errorf("machine power is not configured on this server")
		}
		if !p.stopping {
			// Refuse bad credentials before disconnecting anybody. The caller
			// supplies no coordinates, signal, token, command or URL.
			ctx, cancel := context.WithTimeout(context.Background(), 8*time.Second)
			defer cancel()
			if err := p.provider.Check(ctx); err != nil {
				return nil, fmt.Errorf("machine power provider unavailable; check the server's power configuration")
			}
			p.stopping, p.lastError = true, ""
			go p.stop(nil)
		}
		p.manualStop = true
		return map[string]any{"accepted": true}, nil
	}
}

func (p *machinePower) stop(stillIdle func() bool) {
	// Let the RPC response reach the initiator before closing all clients.
	time.Sleep(time.Second)
	if stillIdle != nil && !stillIdle() {
		p.mu.Lock()
		if !p.manualStop {
			p.stopping = false
			p.mu.Unlock()
			return
		}
		p.mu.Unlock()
	}
	p.disconnect()
	ctx, cancel := context.WithTimeout(context.Background(), time.Minute)
	defer cancel()
	err := p.provider.Stop(ctx)
	if err != nil {
		log.Print("machine power: stop request failed; reconnect to inspect or retry")
		p.mu.Lock()
		p.stopping = false
		p.lastError = "The stop request failed. The machine may still be running."
		p.mu.Unlock()
	}
}

// idlePowerInputs strengthens the advisory predicate before it becomes an
// automatic action. There is no external scheduled wake configured, so ALL
// scheduled jobs keep this machine up, including shell jobs and distant jobs.
func idlePowerInputs(in quiescence.Inputs) quiescence.Inputs {
	for i := range in.Jobs {
		if in.Jobs[i].Running || !in.Jobs[i].NextRun.IsZero() {
			in.Jobs[i].ActionKind = "call"
			in.Jobs[i].Running = true
		}
	}
	return in
}

func (p *machinePower) runIdle(ctx context.Context, read func(context.Context) quiescence.Inputs, dwell, interval time.Duration) {
	if dwell <= 0 {
		return
	}
	mon := quiescence.NewMonitor(quiescence.Tunables{Dwell: dwell, ClientIdleWindow: interval})
	tick := time.NewTicker(interval)
	defer tick.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-tick.C:
			in := idlePowerInputs(read(ctx))
			result := mon.Observe(in)
			p.mu.Lock()
			p.idleState = result
			p.mu.Unlock()
			if !result.Quiescent || p.observeOnly || p.provider == nil {
				continue
			}
			p.mu.Lock()
			if p.stopping || p.lastError != "" {
				p.mu.Unlock()
				continue
			}
			p.mu.Unlock()
			checkCtx, cancel := context.WithTimeout(ctx, 8*time.Second)
			err := p.provider.Check(checkCtx)
			cancel()
			if err != nil {
				continue
			}
			p.mu.Lock()
			if p.stopping {
				p.mu.Unlock()
				continue
			}
			p.stopping, p.manualStop = true, false
			p.mu.Unlock()
			go p.stop(func() bool {
				// Re-read just before the stop, not only before the API preflight.
				return ctx.Err() == nil && mon.Observe(idlePowerInputs(read(ctx))).Quiescent
			})
		}
	}
}

func (p *machinePower) idleMode() string {
	if p.idleTimeout <= 0 {
		return "off"
	}
	if p.observeOnly || p.provider == nil {
		return "observe"
	}
	return "stop"
}

func machineIdleTimeout() time.Duration {
	mode := os.Getenv("WKS_MACHINE_IDLE_MODE")
	if mode == "off" {
		return 0
	}
	if mode != "" && mode != "observe" && mode != "stop" {
		log.Print("machine power: invalid idle mode; use observe, stop, or off")
		return 0
	}
	raw := strings.TrimSpace(os.Getenv("WKS_MACHINE_IDLE_TIMEOUT"))
	if raw == "" || raw == "0" || raw == "off" {
		return 0
	}
	duration, err := time.ParseDuration(raw)
	if err != nil || duration < 10*time.Minute {
		log.Print("machine power: WKS_MACHINE_IDLE_TIMEOUT must be at least 10m (or off); automatic stop disabled")
		return 0
	}
	return duration
}
