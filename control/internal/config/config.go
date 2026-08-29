package config

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"gopkg.in/yaml.v3"
)

// Config holds the control plane's runtime configuration, loaded from app.yaml.
type Config struct {
	Addr       string
	EngineAddr string
	DataDir    string
	Providers  []string
	ChatStream bool
}

type appConfig struct {
	Engine struct {
		GrpcAddr   string `yaml:"grpc_addr"`
		DataDir    string `yaml:"data_dir"`
		ChatStream *bool  `yaml:"chat_stream"`
	} `yaml:"engine"`
	ControlPlane struct {
		Addr       string `yaml:"addr"`
		EngineAddr string `yaml:"engine_addr"`
	} `yaml:"control_plane"`
	Providers []struct {
		Name string `yaml:"name"`
	} `yaml:"providers"`
}

// Load reads app.yaml, expands ${VAR} environment references and ~, and returns
// the control plane's runtime configuration.
func Load() (Config, error) {
	path := configPath()
	raw, err := os.ReadFile(path)
	if err != nil {
		return Config{}, fmt.Errorf("read config %s: %w", path, err)
	}

	var ac appConfig
	if err := yaml.Unmarshal([]byte(os.ExpandEnv(string(raw))), &ac); err != nil {
		return Config{}, fmt.Errorf("parse config %s: %w", path, err)
	}

	cfg := Config{
		Addr:       firstNonEmpty(ac.ControlPlane.Addr, ":9090"),
		EngineAddr: firstNonEmpty(ac.ControlPlane.EngineAddr, ac.Engine.GrpcAddr, "127.0.0.1:50051"),
		DataDir:    expandTilde(firstNonEmpty(ac.Engine.DataDir, "./share/rsmgo")),
		ChatStream: true,
	}

	if ac.Engine.ChatStream != nil {
		cfg.ChatStream = *ac.Engine.ChatStream
	}

	// Allow container runtimes to point the control plane at a different engine
	// hostname without editing the mounted app.yaml.
	if addr := os.Getenv("RSMGO_ENGINE_ADDR"); addr != "" {
		cfg.EngineAddr = addr
	}

	for _, p := range ac.Providers {
		if p.Name != "" {
			cfg.Providers = append(cfg.Providers, p.Name)
		}
	}
	return cfg, nil
}

// configPath resolves the app.yaml location: $RSMGO_CONFIG, then
// ~/.config/rsmgo/app.yaml, then ./app.yaml. This matches the Rust engine's
// resolution order so both sides read the same file.
func configPath() string {
	if p := os.Getenv("RSMGO_CONFIG"); p != "" {
		return p
	}
	if home, err := os.UserHomeDir(); err == nil {
		candidate := filepath.Join(home, ".config", "rsmgo", "app.yaml")
		if _, err := os.Stat(candidate); err == nil {
			return candidate
		}
	}
	return "app.yaml"
}

// expandTilde expands a leading "~/" to the user's home directory.
func expandTilde(path string) string {
	if !strings.HasPrefix(path, "~/") {
		return path
	}
	home, err := os.UserHomeDir()
	if err != nil {
		return path
	}
	return filepath.Join(home, path[2:])
}

func firstNonEmpty(values ...string) string {
	for _, v := range values {
		if v != "" {
			return v
		}
	}
	return ""
}
