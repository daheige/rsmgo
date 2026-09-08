// Package mcp exposes the rsmgo engine's tool set as a Model Context
// Protocol server, so external MCP clients (Claude Desktop, MCP Inspector,
// other agents) can call rsmgo tools over stdio or Streamable HTTP.
package mcp

import (
	"context"
	"encoding/json"
	"log"
	"net/http"
	"time"

	pb "github.com/daheige/rsmgo/pb"
	mcpgo "github.com/modelcontextprotocol/go-sdk/mcp"
)

// ServerName identifies this MCP server in the initialize handshake.
const ServerName = "rsmgo"

// EngineClient is the subset of the engine gRPC client this package needs.
// Declared as an interface so tests can substitute a fake engine.
type EngineClient interface {
	ListTools(ctx context.Context) (*pb.ListToolsResponse, error)
	ListModels(ctx context.Context, provider string) (*pb.ListModelsResponse, error)
	Health(ctx context.Context) (*pb.HealthResponse, error)
	ExecuteTool(ctx context.Context, name, args string) (*pb.ExecuteToolResponse, error)
}

// serverConfig collects optional knobs for NewServer.
type serverConfig struct {
	// toolRefreshInterval is how often the MCP server re-syncs its tool list
	// with the engine. Zero or negative disables background refresh.
	toolRefreshInterval time.Duration
}

// ServerOption customizes NewServer.
type ServerOption func(*serverConfig)

// WithToolRefreshInterval sets how often the server re-syncs its tool list
// with the engine (default 30s). Pass a zero or negative duration to disable
// background refresh; the initial sync still happens at startup.
func WithToolRefreshInterval(d time.Duration) ServerOption {
	return func(c *serverConfig) { c.toolRefreshInterval = d }
}

// NewServer builds an MCP server that proxies every tool registered in the
// engine (including tools imported from external MCP servers).
//
// The tool list is synced at startup and then re-synced in the background
// (every 30s by default, configurable via WithToolRefreshInterval), so the
// control plane picks up engine tool-set changes without a restart; connected
// clients are notified via the standard tools/list_changed notification.
// If the engine is unreachable at startup the server still comes up (with no
// proxy tools) and fills in the tools once the engine answers a later refresh.
//
// The server also exposes MCP resources (rsmgo://tools, rsmgo://providers,
// rsmgo://health) and built-in prompt templates (code_review, explain_code,
// summarize_text).
func NewServer(ec EngineClient, version string, opts ...ServerOption) *mcpgo.Server {
	cfg := serverConfig{toolRefreshInterval: 30 * time.Second}
	for _, opt := range opts {
		opt(&cfg)
	}

	srv := mcpgo.NewServer(&mcpgo.Implementation{Name: ServerName, Version: version}, nil)
	registerResources(srv, ec)
	registerPrompts(srv)
	newToolWatcher(srv, ec, cfg.toolRefreshInterval)
	return srv
}

// addProxyTool registers one engine tool as an MCP tool whose handler forwards
// calls to the engine's ExecuteTool RPC. Returns false (and registers
// nothing) when the tool's parameter schema is unusable.
func addProxyTool(srv *mcpgo.Server, ec EngineClient, info *pb.ToolInfo) bool {
	schema := map[string]any{"type": "object"}
	if info.ParametersSchema != "" {
		parsed := map[string]any{}
		if err := json.Unmarshal([]byte(info.ParametersSchema), &parsed); err != nil {
			log.Printf("mcp: tool %q: invalid parameters schema: %v; using empty object schema", info.Name, err)
		} else {
			schema = parsed
		}
	}
	// The SDK requires tool input schemas to declare type "object"; a missing
	// or non-object type would panic in AddTool, so skip such tools.
	if t, _ := schema["type"].(string); t != "object" {
		log.Printf("mcp: tool %q: parameters schema type is not \"object\"; skipping tool", info.Name)
		return false
	}

	tool := &mcpgo.Tool{
		Name:        info.Name,
		Description: info.Description,
		InputSchema: schema,
	}

	srv.AddTool(tool, func(ctx context.Context, req *mcpgo.CallToolRequest) (*mcpgo.CallToolResult, error) {
		// The raw arguments are forwarded as-is to the engine; the engine
		// validates them against the tool's own schema.
		resp, err := ec.ExecuteTool(ctx, info.Name, string(req.Params.Arguments))
		if err != nil {
			return toolError("engine error: " + err.Error()), nil
		}
		if !resp.Success {
			msg := resp.Error
			if msg == "" {
				msg = "tool execution failed"
			}
			return toolError(msg), nil
		}
		return &mcpgo.CallToolResult{
			Content: []mcpgo.Content{&mcpgo.TextContent{Text: resp.Output}},
		}, nil
	})
	return true
}

// toolError builds a CallToolResult reporting a tool-level error. Per the MCP
// spec, tool errors are reported in the result content (not as protocol
// errors) so the calling LLM can see and self-correct.
func toolError(msg string) *mcpgo.CallToolResult {
	return &mcpgo.CallToolResult{
		Content: []mcpgo.Content{&mcpgo.TextContent{Text: msg}},
		IsError: true,
	}
}

// ServeStdio serves the MCP server over stdin/stdout (newline-delimited JSON)
// until the client disconnects. In this mode nothing may write to stdout, so
// all logging must go to stderr.
func ServeStdio(ctx context.Context, srv *mcpgo.Server) error {
	session, err := srv.Connect(ctx, &mcpgo.StdioTransport{}, nil)
	if err != nil {
		return err
	}
	return session.Wait()
}

// NewStreamableHTTPHandler wraps the MCP server in an HTTP handler speaking
// the Streamable HTTP transport, suitable for mounting on the control plane's
// HTTP server (e.g. at /mcp).
func NewStreamableHTTPHandler(srv *mcpgo.Server) *mcpgo.StreamableHTTPHandler {
	return mcpgo.NewStreamableHTTPHandler(func(*http.Request) *mcpgo.Server {
		return srv
	}, nil)
}
