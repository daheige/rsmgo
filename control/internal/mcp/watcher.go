package mcp

import (
	"context"
	"log"
	"sync"
	"time"

	pb "github.com/daheige/rsmgo/pb"
	mcpgo "github.com/modelcontextprotocol/go-sdk/mcp"
)

// toolWatcher keeps the MCP server's tool list in sync with the engine's
// tool registry. The engine's tool set can change over time (e.g. it was
// restarted with a different mcp_servers config, or a workspace-scoped tool
// was added), and the SDK supports adding/removing tools on a live server
// (clients connected over Streamable HTTP receive notifications/tools/
// list_changed), so the control plane no longer needs a restart for new
// tools to become visible.
type toolWatcher struct {
	srv      *mcpgo.Server
	ec       EngineClient
	interval time.Duration

	mu         sync.Mutex
	registered map[string]bool
}

// newToolWatcher registers the current engine tool set immediately and, when
// interval > 0, starts a background goroutine that refreshes it every
// interval. The goroutine runs for the process lifetime; the control plane
// has no graceful-shutdown path for the MCP server, so no stop channel.
func newToolWatcher(srv *mcpgo.Server, ec EngineClient, interval time.Duration) *toolWatcher {
	w := &toolWatcher{
		srv:        srv,
		ec:         ec,
		interval:   interval,
		registered: make(map[string]bool),
	}
	w.refresh()
	if interval > 0 {
		go w.loop()
	}
	return w
}

func (w *toolWatcher) loop() {
	ticker := time.NewTicker(w.interval)
	defer ticker.Stop()
	for range ticker.C {
		w.refresh()
	}
}

// refresh diffs the engine's current tool list against the tools this
// watcher has registered, adds new ones, and removes stale ones.
func (w *toolWatcher) refresh() {
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()

	resp, err := w.ec.ListTools(ctx)
	if err != nil {
		log.Printf("mcp: tool refresh: engine unreachable: %v; keeping %d registered tools", err, w.len())
		return
	}

	current := make(map[string]*pb.ToolInfo, len(resp.Tools))
	for _, info := range resp.Tools {
		current[info.Name] = info
	}

	w.mu.Lock()
	defer w.mu.Unlock()

	var added, removed []string
	for name, info := range current {
		if w.registered[name] {
			continue
		}
		if addProxyTool(w.srv, w.ec, info) {
			w.registered[name] = true
			added = append(added, name)
		}
	}
	for name := range w.registered {
		if _, ok := current[name]; !ok {
			removed = append(removed, name)
		}
	}
	if len(removed) > 0 {
		w.srv.RemoveTools(removed...)
		for _, name := range removed {
			delete(w.registered, name)
		}
	}

	if len(added) > 0 || len(removed) > 0 {
		log.Printf("mcp: tool set refreshed: added %v, removed %v (total %d)", added, removed, len(w.registered))
	}
}

func (w *toolWatcher) len() int {
	w.mu.Lock()
	defer w.mu.Unlock()
	return len(w.registered)
}
