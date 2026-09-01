package main

// The ask surface for limit-aware routing: `routing.select`, and the model
// catalog the matrix is validated against.
//
// This is the SECOND HALF of cmd/hub/routing.go's wiring, and it is what ends
// that file's documented dormancy. Until something called usage.Latest, the P1
// edge took no readings at all and the hub never polled /usage/report; until
// something constructed routing.Service, routing.yaml was never seeded and
// never read. Both are done here and in main.go.
//
// ROUTING EXPOSES NO WRITE RPC OVER THE BUS, EVER — that, plus the secret gate
// refusing the hub's state directory to fs.write, is the entire security
// argument for the matrix file's `ceilings:` block, and the moment a routing
// write RPC exists the ceiling stops meaning anything.
//
// routing.select STARTS NOTHING AND CHANGES NOTHING. It answers a question.
// What it does do, since the decision became binding, is put its own answer on
// the record: one `routing.decision` event and one line in the append-only
// decision log beside routing.yaml. Neither is a write RPC in the sense above —
// no caller can move a threshold, a profile, a mode or a ceiling through any bus
// method, and causing a record of your own question to be written is the
// opposite kind of act from editing the policy that answers it.
//
// Registered with the LITERAL method name through the caller-aware door, for
// the reason fleet.quiescence is: the answer is caller-dependent (a caller's
// own project directory picks the ceiling that will later govern its spawns),
// and capspec's hub-native guard parses RegisterLocal names out of main.go, so
// a name behind a variable is invisible to it.

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strings"
	"sync"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/bus"
	"github.com/djtouchette/workspacer-hub/internal/busclient"
	"github.com/djtouchette/workspacer-hub/internal/event"
	"github.com/djtouchette/workspacer-hub/internal/routing"
)

// catalogTimeout bounds one catalog probe. `GET /providers/:p/models` BOOTS THE
// CLI to ask it what it serves, so it is slower than any other read this file
// does — and a provider that does not answer must leave its models unvalidated
// rather than hold the matrix load open.
const catalogTimeout = 20 * time.Second

// catalogTTL is how long a provider's answer is reused. The matrix reloads on a
// 30-second tick and re-validates every time; without a cache that would boot
// five CLIs twice a minute forever, which is exactly the ambient cost the usage
// poller's wind-down exists to avoid.
const catalogTTL = 10 * time.Minute

// routingCatalog answers routing.Catalog from the two live catalogs this repo
// already has, and from nothing else.
//
//	claude    `claude.listModels` over the bus — the desktop (or the brain)
//	          answers it, and the answer is that installation's own alias list.
//	the rest  claudemon's `GET /providers/:provider/models`, which boots the
//	          provider CLI and reports id/label/default/effortLevels.
//
// AN ERROR MEANS "COULD NOT ASK", NEVER "THE MODEL IS WRONG". A codex CLI that
// is not installed on this machine says nothing about whether the matrix is
// right, and ValidateAgainstCatalog skips a provider it cannot get an answer
// for. This adapter therefore returns the error rather than an empty list —
// they are different claims and collapsing them would condemn every model on a
// machine where the daemon is simply down.
type routingCatalog struct {
	base string
	self *busclient.Client
	http *http.Client

	mu     sync.Mutex
	cached map[string]catalogEntry
}

type catalogEntry struct {
	models []routing.CatalogModel
	err    error
	at     time.Time
}

func newRoutingCatalog(base string, self *busclient.Client) *routingCatalog {
	return &routingCatalog{
		base:   base,
		self:   self,
		http:   &http.Client{Timeout: catalogTimeout},
		cached: map[string]catalogEntry{},
	}
}

// Models implements routing.Catalog.
func (c *routingCatalog) Models(provider string) ([]routing.CatalogModel, error) {
	provider = strings.ToLower(strings.TrimSpace(provider))
	if provider == "" {
		return nil, fmt.Errorf("no provider named")
	}

	c.mu.Lock()
	if e, ok := c.cached[provider]; ok && time.Since(e.at) < catalogTTL {
		c.mu.Unlock()
		return e.models, e.err
	}
	c.mu.Unlock()

	ctx, cancel := context.WithTimeout(context.Background(), catalogTimeout)
	defer cancel()

	var models []routing.CatalogModel
	var err error
	if provider == "claude" {
		models, err = c.claudeModels(ctx)
	} else {
		models, err = c.providerModels(ctx, provider)
	}

	c.mu.Lock()
	c.cached[provider] = catalogEntry{models: models, err: err, at: time.Now()}
	c.mu.Unlock()
	return models, err
}

// claudeModels reads `claude.listModels` off the bus. The answer's `aliases`
// are the launchable names (opus, sonnet, fable, …) and `seen` are concrete ids
// observed in transcripts; both are accepted, because a matrix may legitimately
// pin `claude-opus-5` rather than the moving alias.
func (c *routingCatalog) claudeModels(ctx context.Context) ([]routing.CatalogModel, error) {
	if c.self == nil {
		return nil, fmt.Errorf("no bus client to ask claude.listModels with")
	}
	raw, err := c.self.Call(ctx, "claude.listModels", map[string]any{})
	if err != nil {
		return nil, fmt.Errorf("claude.listModels: %w", err)
	}
	var body struct {
		Aliases []struct {
			Value string `json:"value"`
		} `json:"aliases"`
		Seen []string `json:"seen"`
	}
	if err := json.Unmarshal(raw, &body); err != nil {
		return nil, fmt.Errorf("claude.listModels: %w", err)
	}
	out := make([]routing.CatalogModel, 0, len(body.Aliases)+len(body.Seen))
	for _, a := range body.Aliases {
		if a.Value != "" {
			out = append(out, routing.CatalogModel{ID: a.Value})
		}
	}
	for _, id := range body.Seen {
		if id != "" {
			out = append(out, routing.CatalogModel{ID: id})
		}
	}
	if len(out) == 0 {
		// An empty-but-successful answer is "I do not know" here, not "claude
		// serves nothing": a fresh install with no transcripts and no seen list
		// answers this way, and failing every claude model against it would be a
		// false alarm on the one shape that means ignorance.
		return nil, fmt.Errorf("claude.listModels answered no models")
	}
	return out, nil
}

func (c *routingCatalog) providerModels(ctx context.Context, provider string) ([]routing.CatalogModel, error) {
	if c.base == "" {
		return nil, fmt.Errorf("no claudemon URL to ask for %s models", provider)
	}
	u := strings.TrimRight(c.base, "/") + "/providers/" + url.PathEscape(provider) + "/models"
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, u, nil)
	if err != nil {
		return nil, err
	}
	resp, err := c.http.Do(req)
	if err != nil {
		return nil, fmt.Errorf("claudemon GET /providers/%s/models: %w", provider, err)
	}
	defer resp.Body.Close()
	if resp.StatusCode >= 400 {
		return nil, fmt.Errorf("claudemon GET /providers/%s/models: %s", provider, resp.Status)
	}
	var body struct {
		Models []struct {
			ID           string   `json:"id"`
			EffortLevels []string `json:"effortLevels"`
		} `json:"models"`
	}
	if err := json.NewDecoder(io.LimitReader(resp.Body, 4<<20)).Decode(&body); err != nil {
		return nil, fmt.Errorf("claudemon GET /providers/%s/models: %w", provider, err)
	}
	if len(body.Models) == 0 {
		return nil, fmt.Errorf("claudemon reports no %s models", provider)
	}
	out := make([]routing.CatalogModel, 0, len(body.Models))
	for _, m := range body.Models {
		if m.ID == "" {
			continue
		}
		out = append(out, routing.CatalogModel{ID: m.ID, EffortLevels: m.EffortLevels})
	}
	return out, nil
}

// ---------------------------------------------------------------------------
// THE HANDLER
// ---------------------------------------------------------------------------

// routingDecisionTopic is the bus event every routing answer publishes. Section
// 37's name, and the only topic this layer has.
//
// The literal lives here, on its own line, because capspec's publish sweep
// (TestEveryPublishedTopicIsClassified) resolves a topic constant by scanning
// the hub's sources for exactly this shape — a topic behind a computed name is
// a topic nobody classified.
const routingDecisionTopic = "routing.decision"

// routingDecisionEvent is the published projection of a Decision, and it is
// deliberately NARROWER than the answer the caller gets.
//
// The topic is open-by-decision — every tier receives it — so what rides it has
// to be worth disclosing to a view-tier phone. The model, the capability, the
// mode and the reasoning are: that is section 35's "promoted from Sol High,
// reason: spend-down, reset in 38m" and it names no credential, no path and no
// argv. The REQUEST's cwd and account do not ride it. They are in the decision
// log, which is a 0600 file in the hub's own state directory, because "which
// project directory is being worked in" is a fact about the user's disk and the
// event plane has no business broadcasting it for an audit trail's benefit.
type routingDecisionEvent struct {
	DecisionID string   `json:"decisionId"`
	TicketID   string   `json:"ticketId,omitempty"`
	Role       string   `json:"role"`
	Capability string   `json:"capability"`
	Base       string   `json:"baseCapability"`
	Profile    string   `json:"profile,omitempty"`
	Provider   string   `json:"provider,omitempty"`
	Model      string   `json:"model,omitempty"`
	Effort     string   `json:"effort,omitempty"`
	Eligible   bool     `json:"eligible"`
	Mode       string   `json:"mode"`
	ModeManual bool     `json:"modeManual"`
	Health     string   `json:"health,omitempty"`
	Reason     []string `json:"reason,omitempty"`
	// CeilingCapped / CeilingMaxCapability report that a DIRECTORY CEILING
	// lowered this answer, and to what. The ceiling's KEY — an absolute project
	// directory — deliberately does NOT ride: this topic is open-by-decision, so
	// a view-tier phone receives it, and "which directory is capped" is a fact
	// about the user's disk. The key is in the decision log, which is 0600 in the
	// hub's own state directory. Without these two, a client watching the event
	// plane would see a capability change with no reason present in the payload.
	CeilingCapped        bool   `json:"ceilingCapped,omitempty"`
	CeilingMaxCapability string `json:"ceilingMaxCapability,omitempty"`
	// Pace / PaceRatio / PaceWindow are the WINDOW-PROGRESS half of the answer:
	// the state (on_track | ahead | overspending | unknown), how far over or
	// under the curve consumption is running, and which window that ratio came
	// off. They ride for the same reason `health` does — a client watching the
	// event plane would otherwise see a mode change to CONSERVE with the reason
	// present only as prose, and "codex 5h at 1.6x" is exactly the caption a
	// fleet display wants. They are aggregate capacity facts about the user's
	// own subscription: no path, no credential, no account key. All three are
	// ABSENT when pacing is switched off, so an event never carries a pace the
	// decision did not use.
	Pace       string  `json:"pace,omitempty"`
	PaceRatio  float64 `json:"paceRatio,omitempty"`
	PaceWindow string  `json:"paceWindow,omitempty"`
	DecidedAt  int64   `json:"decidedAt"`
}

// paceOf projects a decision's pace onto the event's three fields. A decision
// with no pace (pacing off, or nothing readable) contributes nothing, which is
// what keeps `enabled: false` producing the pre-pacing event as well as the
// pre-pacing answer.
func paceOf(d routing.Decision) (state string, ratio float64, window string) {
	p := d.Capacity.Pace
	if p == nil {
		return "", 0, ""
	}
	if !p.Known {
		return string(p.State), 0, ""
	}
	return string(p.State), p.Ratio, p.Window
}

// routingSelect is the `routing.select` handler: read the matrix in force, take
// a usage reading, judge it against THIS instant, answer — and put the answer on
// the record.
//
// The reading is taken through usageWatcher.LatestWithin with a zero freshness
// bound, so every decision has its own document. See usageDecisionMaxAge for
// why a decision does not reuse a cached one, and usageDecisionWait for what
// happens when that document does not arrive.
//
// A usage reading that CANNOT be taken is not an error the caller sees: it is an
// UNKNOWN capacity with the failure named in the reason list, and the matrix
// still answers which capability the role needs. A hub that refused to route
// because claudemon was restarting would be worse than one that routed
// conservatively and said why.
//
// STILL NO WRITE RPC. This handler now publishes an event and appends a line to
// an audit file, and neither of those is a write RPC in the sense that matters:
// no caller can change the matrix, the ceilings, the thresholds or the modes
// through any bus method, which is what makes routing.yaml's `ceilings:` block a
// ceiling. A caller can cause a record OF ITS OWN QUESTION to be written, which
// is the opposite kind of thing.
func routingSelect(svc *routing.Service, usage *usageWatcher, pub func(event.Envelope), logf *routing.DecisionLog) bus.LocalIdentHandler {
	return func(_ bus.CallerIdentity, params json.RawMessage) (any, error) {
		var req routing.Request
		if len(params) > 0 {
			if err := json.Unmarshal(params, &req); err != nil {
				return nil, fmt.Errorf("routing.select: %w", err)
			}
		}
		if strings.TrimSpace(req.Role) == "" {
			return nil, fmt.Errorf("routing.select: role is required — routing answers in ROLES and CAPABILITIES, never in model names (see routing.yaml's roles: block for the vocabulary)")
		}

		// THE SAME CANONICALIZING WALK THE SPAWN GATE USES, and that is the point
		// of borrowing it from internal/bus rather than writing one here. The
		// ceiling lookup on the other side is a LEXICAL ancestor match, so a
		// ceiling looked up on the caller's spelling is a ceiling a symlink walks
		// around — and if routing.select resolved paths even slightly differently
		// from the gate, the two would cap different directories and the
		// advise-then-refuse contradiction would come straight back.
		//
		// An unresolvable or unnamed cwd leaves this EMPTY, which selects the
		// `default` ceiling, exactly as it does at the gate. "We could not resolve
		// it" must not read as "unconstrained" on either side.
		if canonical, ok := bus.CanonicalizeRoot(req.Cwd); ok {
			req.CanonicalCwd = canonical
		}

		ctx, cancel := context.WithTimeout(context.Background(), usageDecisionWait)
		defer cancel()

		snap, snapErr := usage.LatestWithin(ctx, usageDecisionMaxAge)
		d := routing.Select(svc.Matrix(), snap, snapErr, time.Now(), req)
		// Stamped HERE rather than inside Select, which is pure: the id is what a
		// spawn quotes back as `decisionId` and what joins the two rows of the
		// log, so it is minted exactly where a decision becomes a fact.
		d.DecisionID = routing.NewDecisionID()

		logf.Decision(d)
		if pub != nil {
			paceState, paceRatio, paceWindow := paceOf(d)
			pub(event.New(routingDecisionTopic, "routing", routingDecisionEvent{
				DecisionID: d.DecisionID,
				TicketID:   d.TicketID,
				Role:       d.Role,
				Capability: d.Capability,
				Base:       d.BaseCapability,
				Profile:    d.Profile,
				Provider:   d.Provider,
				Model:      d.Model,
				Effort:     d.Effort,
				Eligible:   d.Eligible,
				Mode:       string(d.Mode),
				ModeManual: d.ModeManual,
				Health:     string(d.Capacity.Health),
				Reason:     d.Reason,
				CeilingCapped: d.Ceiling != nil &&
					(d.Ceiling.CapabilityRefused || d.Ceiling.ToolScopeRefused || d.Ceiling.Denied),
				CeilingMaxCapability: ceilingMax(d.Ceiling),
				Pace:                 paceState,
				PaceRatio:            paceRatio,
				PaceWindow:           paceWindow,
				DecidedAt:            d.DecidedAt,
			}))
		}
		return d, nil
	}
}

// ceilingMax is the verdict's max_capability, nil-safe. A named function rather
// than an inline conditional because the event literal above is already long
// enough to hide a nil dereference in.
func ceilingMax(v *routing.CeilingVerdict) string {
	if v == nil {
		return ""
	}
	return v.MaxCapability
}
