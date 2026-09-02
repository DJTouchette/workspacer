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

// availabilityMinInterval rate-limits the DEMAND-DRIVEN half of this file: the
// availability refresh a routing.select kicks off in the background.
//
// The cadence is the ask's, not a timer's, for exactly the reason
// usageSampleIdleAfter exists — nothing should boot a provider CLI on a machine
// where nobody is routing. One refresh at a time (see refreshing), never more
// often than this, and never on the caller's own goroutine: a decision must not
// wait 20 seconds for a CLI that is not installed to fail to answer.
const availabilityMinInterval = 5 * time.Second

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
	// refreshing is the single flight for the background availability refresh,
	// and lastRefresh is what availabilityMinInterval is measured from.
	refreshing  bool
	lastRefresh time.Time
}

type catalogEntry struct {
	models []routing.CatalogModel
	// answered is whether THIS PROVIDER gave us its own answer, even if that
	// answer was an empty list. It is the whole distinction the availability map
	// rests on: `err != nil` is "we could not ask" (unknown, fail open), while
	// answered-with-nothing is "the provider says it can launch nothing"
	// (unavailable, with a reason). Collapsing them is how a claudemon restart
	// would declare every provider dead.
	answered bool
	err      error
	at       time.Time
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
//
// An empty-but-successful answer is returned as an ERROR here, and that is not
// a change of heart about what an empty list means: ValidateAgainstCatalog
// documents "I could not ask" and "the answer was empty" as the same
// instruction — leave this provider's models unvalidated — and a fresh install
// with no transcripts answers claude.listModels exactly that way. The
// AVAILABILITY half needs the two apart, so the distinction is kept on the
// cached entry (see catalogEntry.answered) and flattened here, rather than the
// other way round.
func (c *routingCatalog) Models(provider string) ([]routing.CatalogModel, error) {
	e := c.entry(provider, false)
	if e.err != nil {
		return nil, e.err
	}
	if len(e.models) == 0 {
		return nil, fmt.Errorf("%s answered no models", provider)
	}
	return e.models, nil
}

// entry is the cached probe, with two TTLs by way of one rule: an answer with
// MODELS IN IT is reused for catalogTTL, and every other cached state is
// re-probed whenever `force` asks for it (which is what the background
// availability refresh does, at most every availabilityMinInterval).
//
// The asymmetry is the honest one, and the line it is drawn on is "did this
// probe find something launchable", not "did somebody answer". A provider
// serving models is unlikely to change what it serves within ten minutes. The
// other two states are both states somebody is expected to FIX while the hub
// runs:
//
//	never answered      the daemon was down, or there was no peer to ask. A
//	                    daemon coming back is the ordinary case.
//	answered, no models the CLI is there and can launch nothing, or is not there
//	                    at all. A provider somebody has just installed or logged
//	                    into is exactly the one whose state changes next, and
//	                    holding the old verdict for ten minutes would make an
//	                    installed CLI unroutable for ten minutes after it works.
//
// Both are cheap to be wrong about in the direction of asking again: a forced
// refresh runs in the background, one at a time, on the cadence of somebody
// routing.
func (c *routingCatalog) entry(provider string, force bool) catalogEntry {
	provider = strings.ToLower(strings.TrimSpace(provider))
	if provider == "" {
		return catalogEntry{err: fmt.Errorf("no provider named"), at: time.Now()}
	}

	c.mu.Lock()
	e, ok := c.cached[provider]
	c.mu.Unlock()
	// settled is the one state a forced refresh leaves alone: this provider
	// answered, and it named at least one launchable model.
	settled := e.answered && len(e.models) > 0
	if ok && time.Since(e.at) < catalogTTL && !(force && !settled) {
		return e
	}

	ctx, cancel := context.WithTimeout(context.Background(), catalogTimeout)
	defer cancel()

	var models []routing.CatalogModel
	var answered bool
	var err error
	if provider == "claude" {
		models, answered, err = c.claudeModels(ctx)
	} else {
		models, answered, err = c.providerModels(ctx, provider)
	}

	fresh := catalogEntry{models: models, answered: answered, err: err, at: time.Now()}
	c.mu.Lock()
	c.cached[provider] = fresh
	c.mu.Unlock()
	return fresh
}

// Availability projects the cache onto the map routing.Select takes.
//
// It does NO I/O: it reports what the last probe of each provider found, and a
// provider nobody has probed (or nobody could reach) is simply absent, which is
// the fail-open state routing.ProviderAvailability documents. RefreshAvailability
// is what makes the projection current.
func (c *routingCatalog) Availability() routing.ProviderAvailability {
	c.mu.Lock()
	defer c.mu.Unlock()
	out := routing.ProviderAvailability{}
	for provider, e := range c.cached {
		if !e.answered {
			continue // could not ask — unknown, and unknown is not unavailable
		}
		live := routing.ProviderLiveness{Available: len(e.models) > 0, ObservedAt: e.at.Unix()}
		if !live.Available {
			// WHAT THIS SENTENCE MAY CLAIM is exactly what the probe saw: the
			// CLI ran and listed no launchable model. It may NOT claim the CLI
			// is missing. A missing binary makes claudemon's spawn fail, which
			// is a non-2xx, which is `answered: false`, which never reaches
			// this map at all.
			live.Reason = fmt.Sprintf(
				"%s's CLI ran and reported no launchable model, so there is nothing to start on it right now", provider)
		}
		out[provider] = live
	}
	if len(out) == 0 {
		return nil
	}
	return out
}

// RefreshAvailability re-probes the providers a matrix can actually route to,
// in the BACKGROUND, at most one refresh at a time and no more often than
// availabilityMinInterval.
//
// A provider that answered WITH MODELS stays cached for catalogTTL like any
// other catalog answer. A provider that could not be reached, and a provider
// that answered with nothing, are both asked again: those are the entries whose
// staleness costs something. See entry.
func (c *routingCatalog) RefreshAvailability(providers []string) {
	if len(providers) == 0 {
		return
	}
	c.mu.Lock()
	if c.refreshing || time.Since(c.lastRefresh) < availabilityMinInterval {
		c.mu.Unlock()
		return
	}
	c.refreshing, c.lastRefresh = true, time.Now()
	c.mu.Unlock()

	go func() {
		defer func() {
			c.mu.Lock()
			c.refreshing = false
			c.mu.Unlock()
		}()
		for _, p := range providers {
			c.entry(p, true)
		}
	}()
}

// claudeModels reads `claude.listModels` off the bus. The answer's `aliases`
// are the launchable names (opus, sonnet, fable, …) and `seen` are concrete ids
// observed in transcripts; both are accepted, because a matrix may legitimately
// pin `claude-opus-5` rather than the moving alias.
func (c *routingCatalog) claudeModels(ctx context.Context) ([]routing.CatalogModel, bool, error) {
	if c.self == nil {
		return nil, false, fmt.Errorf("no bus client to ask claude.listModels with")
	}
	raw, err := c.self.Call(ctx, "claude.listModels", map[string]any{})
	if err != nil {
		return nil, false, fmt.Errorf("claude.listModels: %w", err)
	}
	var body struct {
		Aliases []struct {
			Value string `json:"value"`
		} `json:"aliases"`
		Seen []string `json:"seen"`
	}
	if err := json.Unmarshal(raw, &body); err != nil {
		return nil, false, fmt.Errorf("claude.listModels: %w", err)
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
		// false alarm on the one shape that means ignorance. It is therefore NOT
		// `answered` either — this is the one provider whose empty answer cannot
		// be read as unavailability, because the list is assembled from aliases
		// and observed transcripts rather than reported by a running CLI.
		return nil, false, fmt.Errorf("claude.listModels answered no models")
	}
	return out, true, nil
}

// providerModels asks claudemon what this provider can launch.
//
// The middle return says whether THE PROVIDER answered. A 200 carrying an empty
// list is an answer — claudemon booted the CLI (or tried to) and it offers
// nothing — and that is the one shape the availability map reads as
// unavailable. Everything else (no daemon URL, a transport failure, a non-2xx,
// a body that does not decode) is "we could not ask", and routes as it always
// did.
func (c *routingCatalog) providerModels(ctx context.Context, provider string) ([]routing.CatalogModel, bool, error) {
	if c.base == "" {
		return nil, false, fmt.Errorf("no claudemon URL to ask for %s models", provider)
	}
	u := strings.TrimRight(c.base, "/") + "/providers/" + url.PathEscape(provider) + "/models"
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, u, nil)
	if err != nil {
		return nil, false, err
	}
	resp, err := c.http.Do(req)
	if err != nil {
		return nil, false, fmt.Errorf("claudemon could not be reached for %s models (GET /providers/%s/models: %v), so nothing is known about this provider", provider, provider, err)
	}
	defer resp.Body.Close()
	if resp.StatusCode >= 400 {
		// THE DAEMON IS UP AND THIS PROBE FAILED, which is the shape a missing
		// CLI produces: claudemon tries to spawn the binary, the spawn fails,
		// and the endpoint answers 502. It is still UNKNOWN rather than
		// unavailable, because the same status covers a daemon that is starting
		// up, a provider it does not implement, and a spawn that timed out. The
		// two are separated in the WORDS so an operator reading a validation
		// issue can tell them apart, and joined in the BEHAVIOUR because both
		// fail open.
		return nil, false, fmt.Errorf("claudemon answered %s for %s's model catalog (GET /providers/%s/models), so this provider's models could not be listed", resp.Status, provider, provider)
	}
	var body struct {
		Models []struct {
			ID           string   `json:"id"`
			EffortLevels []string `json:"effortLevels"`
		} `json:"models"`
	}
	if err := json.NewDecoder(io.LimitReader(resp.Body, 4<<20)).Decode(&body); err != nil {
		return nil, false, fmt.Errorf("claudemon GET /providers/%s/models: %w", provider, err)
	}
	if len(body.Models) == 0 {
		return nil, true, nil
	}
	out := make([]routing.CatalogModel, 0, len(body.Models))
	for _, m := range body.Models {
		if m.ID == "" {
			continue
		}
		out = append(out, routing.CatalogModel{ID: m.ID, EffortLevels: m.EffortLevels})
	}
	return out, true, nil
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
	Pace string `json:"pace,omitempty"`
	// PaceRatio is a POINTER, not a plain float64 with omitempty: a known
	// on_track ratio can legitimately be exactly 0.0 (nothing used yet), and
	// `omitempty` on a bare float64 drops a real zero exactly as readily as an
	// absent one. Only `Known` decides whether this rides; nil means the same
	// thing 0 used to mean by accident and now means on purpose.
	PaceRatio  *float64 `json:"paceRatio,omitempty"`
	PaceWindow string   `json:"paceWindow,omitempty"`
	// FellOverFrom is the capability's PRIMARY pairing, present only when this
	// answer took one of that capability's `alternatives:` instead. It rides for
	// the same reason `health` and the ceiling fields do: without it a client
	// watching the event plane sees a reviewer come up on the implementer's own
	// family with the explanation present only as prose. It is a provider and a
	// model out of the operator's own routing.yaml — the same class of fact as
	// `provider` and `model`, which already ride — and it names no path, no
	// account and no credential.
	FellOverFrom *routing.Assignment `json:"fellOverFrom,omitempty"`
	// EffortStep is what the mode's `effort_step` did to this answer — the
	// declared effort, the effort it actually runs at, and the one sentence
	// explaining both. It rides for the same reason `health` and the ceiling
	// fields do: `effort` alone changes on the wire with nothing in the payload
	// saying a routing mode moved it, and "Sol, thinking one notch less because
	// the five-hour window is running ahead" is exactly the caption a fleet
	// display wants. ABSENT unless a step was armed, so an event from a matrix
	// with `effort_step: 0` is the pre-stepping event byte for byte.
	EffortStep *routing.EffortStep `json:"effortStep,omitempty"`
	DecidedAt  int64               `json:"decidedAt"`
}

// availabilitySource is the live-launchability half of the two facts Select
// takes from outside itself (the other is the usage snapshot).
//
// An INTERFACE, and nil-safe at the call site, because the harness and every
// unit test construct this handler without a catalog: no catalog means no
// availability answers, which means the map is empty, which means every provider
// is unknown and routing behaves exactly as it did before this existed.
type availabilitySource interface {
	// Availability is a cheap projection of the last probe of each provider.
	Availability() routing.ProviderAvailability
	// RefreshAvailability re-probes, in the background, and returns immediately.
	RefreshAvailability(providers []string)
}

// paceOf projects a decision's pace onto the event's three fields. A decision
// with no pace (pacing off, or nothing readable) contributes nothing, which is
// what keeps `enabled: false` producing the pre-pacing event as well as the
// pre-pacing answer. ratio is nil exactly when the pace is not Known, so a
// genuine 0.0 ratio still rides the event rather than being read as absent.
//
// It reads d.EffectiveCapacity() rather than d.Capacity for the same reason
// the event's Health field below does: a fallover or a cross-provider mode
// shift can land the answer on a provider other than the one Capacity was
// read for, and a pace caption is exactly as misattributed as a health one
// would be if it kept quoting the subject's own window-progress reading.
func paceOf(d routing.Decision) (state string, ratio *float64, window string) {
	p := d.EffectiveCapacity().Pace
	if p == nil {
		return "", nil, ""
	}
	if !p.Known {
		return string(p.State), nil, ""
	}
	r := p.Ratio
	return string(p.State), &r, p.Window
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
func routingSelect(svc *routing.Service, usage *usageWatcher, avail availabilitySource, pub func(event.Envelope), logf *routing.DecisionLog) bus.LocalIdentHandler {
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

		// LIVE PROVIDER AVAILABILITY, read here and refreshed here, because this
		// is the edge: Select is pure and must be handed the fact rather than
		// fetching it. The refresh is kicked BEFORE the read and does not block —
		// it boots provider CLIs, which is the slowest thing this file can do —
		// so a decision acts on the last probe and the next one acts on this one.
		// The first decision after a hub start therefore routes with an empty
		// map, which is the fail-open state and the same answer this hub gave
		// before availability existed.
		matrix := svc.Matrix()
		var liveness routing.ProviderAvailability
		if avail != nil {
			avail.RefreshAvailability(matrix.RoutableProviders())
			liveness = avail.Availability()
		}

		d := routing.Select(matrix, snap, snapErr, liveness, time.Now(), req)
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
				// EffectiveCapacity, NOT Capacity: Capacity is always the
				// SUBJECT's reading (step 4), and a fallover or a
				// cross-provider mode shift is free to land the answer on a
				// different provider. Publishing Capacity.Health unconditionally
				// used to ship `provider: codex, health: red` on a decision that
				// fell over FROM a red claude TO a green codex — the primary's
				// health, misattributed to the provider actually running the
				// work.
				Health: string(d.EffectiveCapacity().Health),
				Reason: d.Reason,
				CeilingCapped: d.Ceiling != nil &&
					(d.Ceiling.CapabilityRefused || d.Ceiling.ToolScopeRefused || d.Ceiling.Denied),
				CeilingMaxCapability: ceilingMax(d.Ceiling),
				Pace:                 paceState,
				PaceRatio:            paceRatio,
				PaceWindow:           paceWindow,
				FellOverFrom:         d.FellOverFrom,
				EffortStep:           d.EffortStep,
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
