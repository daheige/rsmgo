package mcp

import (
	"context"
	"strings"
	"testing"
	"time"

	pb "github.com/daheige/rsmgo/pb"
	mcpgo "github.com/modelcontextprotocol/go-sdk/mcp"
)

// connectInMemory pairs the server with an in-memory client session and
// returns the client session.
func connectInMemory(t *testing.T, srv *mcpgo.Server) *mcpgo.ClientSession {
	t.Helper()
	ctx := context.Background()
	serverTransport, clientTransport := mcpgo.NewInMemoryTransports()
	serverSession, err := srv.Connect(ctx, serverTransport, nil)
	if err != nil {
		t.Fatalf("server connect: %v", err)
	}
	t.Cleanup(func() { serverSession.Close() })

	client := mcpgo.NewClient(&mcpgo.Implementation{Name: "test", Version: "v0"}, nil)
	clientSession, err := client.Connect(ctx, clientTransport, nil)
	if err != nil {
		t.Fatalf("client connect: %v", err)
	}
	t.Cleanup(func() { clientSession.Close() })
	return clientSession
}

// waitForTools polls the client's tool list until it matches want (or the
// deadline passes), accommodating the watcher's asynchronous refresh.
func waitForTools(t *testing.T, cs *mcpgo.ClientSession, want []string) {
	t.Helper()
	deadline := time.Now().Add(3 * time.Second)
	for {
		result, err := cs.ListTools(context.Background(), nil)
		if err == nil {
			got := make(map[string]bool, len(result.Tools))
			for _, tool := range result.Tools {
				got[tool.Name] = true
			}
			match := len(got) == len(want)
			for _, name := range want {
				if !got[name] {
					match = false
				}
			}
			if match {
				return
			}
		}
		if time.Now().After(deadline) {
			t.Fatalf("timed out waiting for tools %v; last list err=%v", want, err)
		}
		time.Sleep(25 * time.Millisecond)
	}
}

func TestToolWatcherHotReload(t *testing.T) {
	fake := &fakeEngine{tools: []*pb.ToolInfo{
		{Name: "tool_a", ParametersSchema: `{"type":"object"}`},
	}}
	srv := NewServer(fake, "test-version", WithToolRefreshInterval(20*time.Millisecond))
	cs := connectInMemory(t, srv)

	waitForTools(t, cs, []string{"tool_a"})

	// Simulate the engine gaining a new tool and losing an old one; the
	// watcher must reflect both without restarting the control plane.
	fake.tools = []*pb.ToolInfo{
		{Name: "tool_b", ParametersSchema: `{"type":"object"}`},
	}
	waitForTools(t, cs, []string{"tool_b"})

	// The new tool must actually be callable through the proxy.
	result, err := cs.CallTool(context.Background(), &mcpgo.CallToolParams{Name: "tool_b"})
	if err != nil {
		t.Fatalf("call tool_b: %v", err)
	}
	if result.IsError || len(fake.called) == 0 || fake.called[len(fake.called)-1] != "tool_b" {
		t.Fatalf("tool_b did not reach the engine: called=%v result=%+v", fake.called, result)
	}
}

func TestResourcesListAndRead(t *testing.T) {
	fake := &fakeEngine{tools: []*pb.ToolInfo{
		{Name: "read_file", Description: "Read a file", ParametersSchema: `{"type":"object"}`},
	}}
	srv := NewServer(fake, "test-version", WithToolRefreshInterval(0))
	cs := connectInMemory(t, srv)

	ctx := context.Background()
	list, err := cs.ListResources(ctx, nil)
	if err != nil {
		t.Fatalf("list resources: %v", err)
	}
	uris := make(map[string]bool, len(list.Resources))
	for _, r := range list.Resources {
		uris[r.URI] = true
	}
	for _, want := range []string{"rsmgo://tools", "rsmgo://providers", "rsmgo://health"} {
		if !uris[want] {
			t.Fatalf("resource %s missing from list %v", want, uris)
		}
	}

	read, err := cs.ReadResource(ctx, &mcpgo.ReadResourceParams{URI: "rsmgo://health"})
	if err != nil {
		t.Fatalf("read health: %v", err)
	}
	if len(read.Contents) != 1 || !strings.Contains(read.Contents[0].Text, `"status": "ok"`) {
		t.Fatalf("unexpected health contents: %+v", read.Contents)
	}

	read, err = cs.ReadResource(ctx, &mcpgo.ReadResourceParams{URI: "rsmgo://tools"})
	if err != nil {
		t.Fatalf("read tools: %v", err)
	}
	if !strings.Contains(read.Contents[0].Text, "read_file") {
		t.Fatalf("tools resource does not contain engine tool: %q", read.Contents[0].Text)
	}
}

func TestPromptsListAndGet(t *testing.T) {
	fake := &fakeEngine{}
	srv := NewServer(fake, "test-version", WithToolRefreshInterval(0))
	cs := connectInMemory(t, srv)

	ctx := context.Background()
	list, err := cs.ListPrompts(ctx, nil)
	if err != nil {
		t.Fatalf("list prompts: %v", err)
	}
	names := make(map[string]bool, len(list.Prompts))
	for _, p := range list.Prompts {
		names[p.Name] = true
	}
	for _, want := range []string{"code_review", "explain_code", "summarize_text"} {
		if !names[want] {
			t.Fatalf("prompt %s missing from list %v", want, names)
		}
	}

	got, err := cs.GetPrompt(ctx, &mcpgo.GetPromptParams{
		Name:      "code_review",
		Arguments: map[string]string{"code": "fn main() {}", "language": "rust"},
	})
	if err != nil {
		t.Fatalf("get prompt: %v", err)
	}
	if len(got.Messages) != 1 {
		t.Fatalf("expected 1 message, got %d", len(got.Messages))
	}
	text, ok := got.Messages[0].Content.(*mcpgo.TextContent)
	if !ok {
		t.Fatalf("content is %T, want *TextContent", got.Messages[0].Content)
	}
	if !strings.Contains(text.Text, "fn main() {}") || !strings.Contains(text.Text, "code review") {
		t.Fatalf("prompt text missing payload: %q", text.Text)
	}
}
