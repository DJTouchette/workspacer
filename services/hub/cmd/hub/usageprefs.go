package main

// The two RPCs behind Settings → "Usage schedule".
//
// They sit BESIDE usage.report rather than inside it. usage.report's
// no-parameter contract is machine-pinned in three places — the handler itself,
// capspec's composition note ("it accepts no caller values"), and
// usagereport_test.go — and the schedule is hub-side state the caller does not
// supply, exactly like the installed matrix. So the projection still turns on
// nothing the caller sent, and the preference travels on its own pair of
// methods with their own gate.

import (
	"encoding/json"
	"fmt"

	"github.com/djtouchette/workspacer-hub/internal/bus"
	"github.com/djtouchette/workspacer-hub/internal/usageprefs"
)

// usagePrefsTrusted is the WRITE gate, and only the write gate.
//
// Modelled on jobsTrusted, and always invoked with the capability's own literal
// name for the same reason: capspec's composition bearings verify this gate by
// grepping for exactly that call shape, so a name behind a variable is a gate
// no test can see. The READ side is deliberately not gated here — it is on
// authtoken's view tier alongside usage.report, because a client that may see
// the pace must be able to see which week the pace was computed against or the
// number is unexplainable.
func usagePrefsTrusted(method string, c bus.CallerIdentity) error {
	if !c.IsTrusted() {
		return fmt.Errorf("%s requires host authority", method)
	}
	return nil
}

// usagePacingScheduleView is the wire shape both methods answer with.
type usagePacingScheduleView struct {
	// Schedule is "five_day", "seven_day", or "" for "nobody has chosen, so
	// routing.yaml answers". The empty string is a real answer, not a missing
	// field, so it is not omitempty.
	Schedule string `json:"schedule"`
	// Configurable is false when this hub was started with no preference file,
	// so a client can disable the control instead of offering a save that
	// cannot persist.
	Configurable bool `json:"configurable"`
}

func usagePacingScheduleOf(prefs *usageprefs.Store) usagePacingScheduleView {
	return usagePacingScheduleView{Schedule: prefs.Schedule(), Configurable: prefs.Path() != ""}
}

// usagePacingSchedule answers the current preference. No parameters.
func usagePacingSchedule(prefs *usageprefs.Store) bus.LocalIdentHandler {
	return func(_ bus.CallerIdentity, params json.RawMessage) (any, error) {
		var args map[string]json.RawMessage
		if len(params) > 0 {
			if err := json.Unmarshal(params, &args); err != nil {
				return nil, fmt.Errorf("usage.pacingSchedule: no parameters accepted")
			}
		}
		if len(args) != 0 {
			return nil, fmt.Errorf("usage.pacingSchedule: no parameters accepted")
		}
		return usagePacingScheduleOf(prefs), nil
	}
}

// usageSetPacingSchedule stores the preference. Trusted-only.
//
// It answers with the STORED value rather than an `{ok: true}`, so a client
// renders what the hub actually holds; a refused or failed write returns an
// error and changes nothing, which is what makes "saved" mean saved.
func usageSetPacingSchedule(prefs *usageprefs.Store) bus.LocalIdentHandler {
	return func(c bus.CallerIdentity, params json.RawMessage) (any, error) {
		if err := usagePrefsTrusted("usage.setPacingSchedule", c); err != nil {
			return nil, err
		}
		var args struct {
			Schedule string `json:"schedule"`
		}
		if len(params) > 0 {
			if err := json.Unmarshal(params, &args); err != nil {
				return nil, fmt.Errorf("usage.setPacingSchedule: %w", err)
			}
		}
		if _, err := prefs.Set(args.Schedule); err != nil {
			return nil, err
		}
		return usagePacingScheduleOf(prefs), nil
	}
}
