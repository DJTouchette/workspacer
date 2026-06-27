package main

import (
	"context"
	"encoding/json"
	"testing"
)

func TestTerminalInputBytesPath(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := newRegistry(newClaudemonClient(srv.URL))

	// Raw keystrokes go via bytes_b64...
	if _, err := reg.handle(context.Background(), "sessions.terminalInput",
		json.RawMessage(`{"sessionId":"s1","bytesB64":"AQI="}`)); err != nil {
		t.Fatal(err)
	}
	in := rec.calls("/sessions/s1/input")
	if len(in) != 1 || in[0].body["bytes_b64"] != "AQI=" {
		t.Fatalf("expected bytes_b64 forwarded, got %+v", in)
	}

	// ...plain data still uses the text path.
	if _, err := reg.handle(context.Background(), "sessions.terminalInput",
		json.RawMessage(`{"sessionId":"s2","data":"hi"}`)); err != nil {
		t.Fatal(err)
	}
	in2 := rec.calls("/sessions/s2/input")
	if len(in2) != 1 || in2[0].body["text"] != "hi" {
		t.Fatalf("expected text forwarded, got %+v", in2)
	}
}
