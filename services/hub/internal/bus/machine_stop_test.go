package bus

import (
	"context"
	"testing"
	"time"

	"github.com/coder/websocket"
)

func TestMachineStopClosesClientsWithExplicitPauseCode(t *testing.T) {
	url, srv := rpcServerWith(t)
	client := dialClient(t, url)
	// A round trip proves the connection has been inserted into the router.
	client.registerMethods()
	provider := dialClient(t, url)
	provider.registerMethods("test.runningProvider")
	done := make(chan struct{})
	go func() { srv.DisconnectForMachineStop(); close(done) }()
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()
	for {
		_, _, err := client.ws.Read(ctx)
		if err == nil {
			continue
		}
		if websocket.CloseStatus(err) != websocket.StatusCode(4001) {
			t.Fatalf("client would retry a normal disconnect: %v", err)
		}
		break
	}
	select {
	case <-done:
		if !contains(provider.registerMethods("test.runningProvider"), "test.runningProvider") {
			t.Fatal("provider drain was interrupted")
		}
	case <-ctx.Done():
		t.Fatal("stop disconnect did not complete")
	}
}
