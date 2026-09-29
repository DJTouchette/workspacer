package quiescence

import (
	"encoding/json"
	"errors"
	"os"
	"reflect"
	"sort"
	"testing"
	"time"
)

func TestPortableFleetQuiescenceContract(t *testing.T) {
	var data struct {
		Cases        []map[string]json.RawMessage `json:"cases"`
		MonitorCases []map[string]json.RawMessage `json:"monitorCases"`
	}
	bytes, e := os.ReadFile("../../../../contracts/fleet-quiescence-cases.json")
	if e != nil {
		t.Fatal(e)
	}
	if e = json.Unmarshal(bytes, &data); e != nil {
		t.Fatal(e)
	}
	get := func(raw json.RawMessage) string { var s string; _ = json.Unmarshal(raw, &s); return s }
	now := time.UnixMilli(1000000)
	for _, c := range data.Cases {
		t.Run(get(c["name"]), func(t *testing.T) {
			rows, err := ParseSessions("", c["sessions"])
			if s := get(c["sessionsError"]); s != "" {
				err = errors.New(s)
			}
			in := Inputs{Now: now, Sessions: rows, SessionsErr: err}
			tun := DefaultTunables()
			_ = json.Unmarshal(c["keepJobsAwake"], &tun.KeepJobsAwake)
			var clients []struct {
				Label string `json:"label"`
				Idle  int64  `json:"idleForMs"`
			}
			_ = json.Unmarshal(c["clients"], &clients)
			for _, c := range clients {
				in.Clients = append(in.Clients, Client{Label: c.Label, LastActive: now.Add(-time.Duration(c.Idle) * time.Millisecond)})
			}
			var jobs []struct {
				ID      string `json:"id"`
				Kind    string `json:"kind"`
				Next    *int64 `json:"nextInMs"`
				Running bool   `json:"running"`
			}
			_ = json.Unmarshal(c["jobs"], &jobs)
			for _, j := range jobs {
				job := Job{ID: j.ID, ActionKind: j.Kind, Running: j.Running}
				if j.Next != nil {
					job.NextRun = now.Add(time.Duration(*j.Next) * time.Millisecond)
				}
				in.Jobs = append(in.Jobs, job)
			}
			var peers []struct {
				Name     string          `json:"name"`
				Error    string          `json:"error"`
				Sessions json.RawMessage `json:"sessions"`
			}
			_ = json.Unmarshal(c["peers"], &peers)
			for _, p := range peers {
				if p.Error == "" {
					sessions, err := ParseSessions(p.Name, p.Sessions)
					if err != nil {
						p.Error = err.Error()
					} else {
						in.Sessions = append(in.Sessions, sessions...)
					}
				}
				in.Peers = append(in.Peers, Peer{Name: p.Name, Err: p.Error})
			}
			var want []string
			_ = json.Unmarshal(c["expect"], &want)
			got := []string{}
			for _, b := range Evaluate(in, tun) {
				got = append(got, b.Kind)
			}
			sort.Strings(want)
			sort.Strings(got)
			if !reflect.DeepEqual(got, want) {
				t.Fatalf("got %v want %v", got, want)
			}
		})
	}
	for _, c := range data.MonitorCases {
		t.Run(get(c["name"]), func(t *testing.T) {
			var dwell, gap int64
			_ = json.Unmarshal(c["dwellMs"], &dwell)
			_ = json.Unmarshal(c["maxGapMs"], &gap)
			mon := NewMonitor(Tunables{Dwell: time.Duration(dwell) * time.Millisecond, MaxSampleGap: time.Duration(gap) * time.Millisecond})
			var samples []struct {
				At       int64           `json:"at"`
				Sessions json.RawMessage `json:"sessions"`
			}
			_ = json.Unmarshal(c["samples"], &samples)
			var result Result
			for _, s := range samples {
				rows, e := ParseSessions("", s.Sessions)
				result = mon.Observe(Inputs{Now: time.UnixMilli(s.At), Sessions: rows, SessionsErr: e})
			}
			if raw, ok := c["latestAt"]; ok {
				var at int64
				_ = json.Unmarshal(raw, &at)
				mon.SetClock(func() time.Time { return time.UnixMilli(at) })
				result = mon.Latest()
			}
			var want struct {
				Quiescent bool     `json:"quiescent"`
				Since     *int64   `json:"since"`
				Calm      int64    `json:"calmSeconds"`
				Kinds     []string `json:"kinds"`
			}
			_ = json.Unmarshal(c["expect"], &want)
			got := []string{}
			for _, b := range result.Blockers {
				got = append(got, b.Kind)
			}
			if result.Quiescent != want.Quiescent || !reflect.DeepEqual(result.Since, want.Since) || result.CalmSeconds != want.Calm || !reflect.DeepEqual(got, want.Kinds) {
				t.Fatalf("got %+v (%v) want %+v", result, got, want)
			}
		})
	}
}
