package main

import (
	"log"
	"path/filepath"

	"github.com/daheige/rsmgo/control/internal/api"
	"github.com/daheige/rsmgo/control/internal/config"
	"github.com/daheige/rsmgo/control/internal/engine"
	"github.com/daheige/rsmgo/control/internal/session"
)

func main() {
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
	uploadDir := filepath.Join(cfg.DataDir, "uploads")

	server := api.NewServer(engineClient, sessionStore, cfg.Providers, uploadDir)
	log.Printf("rsmgo control plane listening on %s", cfg.Addr)
	if err := server.Run(cfg.Addr); err != nil {
		log.Fatalf("server error: %v", err)
	}
}
