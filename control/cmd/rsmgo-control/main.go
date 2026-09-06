package main

import (
	"context"
	"log"
	"os"
	"os/signal"
	"syscall"

	"github.com/daheige/rsmgo/control/internal/api"
	"github.com/daheige/rsmgo/control/internal/config"
	"github.com/daheige/rsmgo/control/internal/engine"
	"github.com/daheige/rsmgo/control/internal/mcp"
	"github.com/daheige/rsmgo/control/internal/session"
)

// version reported by the MCP server in the initialize handshake.
const version = "0.1.0"

func main() {
	log.SetFlags(log.LstdFlags | log.Lmicroseconds)

	// `rsmgo-control mcp` serves the MCP protocol over stdio for local MCP
	// clients such as Claude Desktop. Everything else starts the HTTP control
	// plane (which also exposes MCP over Streamable HTTP at /mcp).
	if len(os.Args) > 1 && os.Args[1] == "mcp" {
		runMCPStdio()
		return
	}

	cfg, err := config.Load()
	if err != nil {
		log.Fatalf("failed to load config: %v", err)
	}

	engineClient, err := engine.NewClient(cfg.EngineAddr)
	if err != nil {
		log.Fatalf("failed to connect to engine: %v", err)
	}
	defer engineClient.Close()

	sessionStore := session.NewStore(cfg.DataDir)

	server := api.NewServer(engineClient, sessionStore, cfg.Providers, cfg.DataDir, cfg.ChatStream)

	// Expose the engine's tools as an MCP server over Streamable HTTP.
	mcpServer := mcp.NewServer(engineClient, version)
	server.MountMCP(mcp.NewStreamableHTTPHandler(mcpServer))

	log.Printf("rsmgo control plane listening on %s (engine=%s data_dir=%s providers=%d)",
		cfg.Addr, cfg.EngineAddr, cfg.DataDir, len(cfg.Providers))
	if err := server.Run(cfg.Addr); err != nil {
		log.Fatalf("server error: %v", err)
	}
}

// runMCPStdio serves MCP over stdin/stdout. In this mode stdout carries only
// protocol messages, so all logging goes to stderr (the default for log).
func runMCPStdio() {
	cfg, err := config.Load()
	if err != nil {
		log.Fatalf("failed to load config: %v", err)
	}

	engineClient, err := engine.NewClient(cfg.EngineAddr)
	if err != nil {
		log.Fatalf("failed to connect to engine: %v", err)
	}
	defer engineClient.Close()

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()

	log.Printf("rsmgo MCP server (stdio) starting, proxying engine at %s", cfg.EngineAddr)
	if err := mcp.ServeStdio(ctx, mcp.NewServer(engineClient, version)); err != nil {
		log.Fatalf("mcp stdio server error: %v", err)
	}
}
