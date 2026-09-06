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
	ExecuteTool(ctx context.Context, name, args string) (*pb.ExecuteToolResponse, error)
}

// NewServer builds an MCP server that proxies every tool registered in the
// engine (including tools imported from external MCP servers).
//
// Tool definitions are fetched once at startup. If the engine is unreachable,
// the server is still returned but has no tools registered — a control plane
// restart after the engine recovers repopulates the list.
func NewServer(ec EngineClient, version string) *mcpgo.Server {
	srv := mcpgo.NewServer(&mcpgo.Implementation{Name: ServerName, Version: version}, nil)
	registerEngineTools(srv, ec)
	return srv
}

// registerEngineTools fetches the engine's tool list and registers a proxy
// handler for each tool.
func registerEngineTools(srv *mcpgo.Server, ec EngineClient) {
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()

	resp, err := ec.ListTools(ctx)
	if err != nil {
		log.Printf("mcp: failed to list engine tools (engine unreachable?): %v; starting with no tools", err)
		return
	}
	for _, info := range resp.Tools {
		if !addProxyTool(srv, ec, info) {
			continue
		}
	}
	log.Printf("mcp: registered %d tools from engine", len(resp.Tools))
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
