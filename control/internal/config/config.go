package config

import (
	"fmt"
	"os"
	"path/filepath"

	"gopkg.in/yaml.v3"
)

// Config holds the control plane's runtime configuration, loaded from app.yaml.
type Config struct {
	Addr       string
	EngineAddr string
	DataDir    string
	Providers  []string
}

type appConfig struct {
	Engine struct {
		GrpcAddr string `yaml:"grpc_addr"`
		DataDir  string `yaml:"data_dir"`
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
		DataDir:    firstNonEmpty(ac.Engine.DataDir, "./share/rsmgo"),
	}

	for _, p := range ac.Providers {
		if p.Name != "" {
			cfg.Providers = append(cfg.Providers, p.Name)
		}
	}
	return cfg, nil
}

// configPath resolves the app.yaml location: $RSMGO_CONFIG, then ./app.yaml,
// then ~/.config/rsmgo/app.yaml.
func configPath() string {
	if p := os.Getenv("RSMGO_CONFIG"); p != "" {
		return p
	}
	if _, err := os.Stat("app.yaml"); err == nil {
		return "app.yaml"
	}
	if home, err := os.UserHomeDir(); err == nil {
		candidate := filepath.Join(home, ".config", "rsmgo", "app.yaml")
		if _, err := os.Stat(candidate); err == nil {
			return candidate
		}
	}
	return "app.yaml"
}

func firstNonEmpty(values ...string) string {
	for _, v := range values {
		if v != "" {
			return v
		}
	}
	return ""
}
