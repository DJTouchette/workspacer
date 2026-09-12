package federation

import (
	"context"
	"encoding/json"
	"sync"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/event"
)

// Controller swaps peer links without restarting the local hub or its workers.
// Existing callers keep their manager pointer until their in-flight call ends;
// cancelled links report an ordinary connection loss, never route elsewhere.
type Controller struct {
	mu      sync.RWMutex
	pub     Publisher
	ctx     context.Context
	current *Manager
	cancels map[string]context.CancelFunc
}

func NewController(ctx context.Context, pub Publisher, peers []Peer) (*Controller, error) {
	controller := &Controller{pub: pub, ctx: ctx}
	if err := controller.Replace(peers); err != nil {
		return nil, err
	}
	return controller, nil
}
func (c *Controller) Replace(peers []Peer) error {
	next, err := New(c.pub, peers)
	if err != nil {
		return err
	}
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.cancels == nil {
		c.cancels = map[string]context.CancelFunc{}
	}
	previous := map[string]*link{}
	if c.current != nil {
		for _, link := range c.current.links {
			previous[link.peer.Name] = link
		}
	}
	stop := func(name string, old *link) {
		if cancel := c.cancels[name]; cancel != nil {
			cancel()
			delete(c.cancels, name)
		}
		old.mu.Lock()
		last := old.lastSeen
		old.mu.Unlock()
		c.pub.Publish(event.New("hub.peer.disconnected", "federation", map[string]any{"peer": name, "lastSeen": last.UTC().Format(time.RFC3339)}))
	}
	for i, link := range next.links {
		old := previous[link.peer.Name]
		delete(previous, link.peer.Name)
		if old != nil && old.peer == link.peer {
			next.links[i] = old
			continue
		}
		if old != nil {
			stop(link.peer.Name, old)
		}
		linkCtx, cancel := context.WithCancel(c.ctx)
		c.cancels[link.peer.Name] = cancel
		go link.run(linkCtx)
	}
	for name, old := range previous {
		stop(name, old)
	}
	c.current = next

	return nil
}
func (c *Controller) manager() *Manager                { c.mu.RLock(); defer c.mu.RUnlock(); return c.current }
func (c *Controller) Peers() []string                  { return c.manager().Peers() }
func (c *Controller) PeersInfo() []PeerInfo            { return c.manager().PeersInfo() }
func (c *Controller) HasPeer(name string) bool         { return c.manager().HasPeer(name) }
func (c *Controller) DispatchEnabled(name string) bool { return c.manager().DispatchEnabled(name) }
func (c *Controller) Forward(ctx context.Context, peer, method string, params json.RawMessage) (json.RawMessage, error) {
	return c.manager().Forward(ctx, peer, method, params)
}
