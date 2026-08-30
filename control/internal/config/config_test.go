package config

import (
	"os"
	"path/filepath"
	"testing"
)

func TestLoadExpandsEnvVars(t *testing.T) {
	t.Setenv("RSMGO_TEST_API_KEY", "secret_from_env")

	dir := t.TempDir()
	path := filepath.Join(dir, "app.yaml")
	content := `
app:
  name: rsmgo
  version: 0.1.0
engine:
  grpc_addr: "127.0.0.1:50051"
  http_addr: "127.0.0.1:8080"
  data_dir: "./share/rsmgo"
providers:
  - name: deepseek
    api_key: "${RSMGO_TEST_API_KEY}"
    base_url: "https://api.deepseek.com"
    default_model: "deepseek-chat"
tools:
  enabled:
    - read_file
control_plane:
  addr: ":9090"
  engine_addr: "127.0.0.1:50051"
`
	if err := os.WriteFile(path, []byte(content), 0644); err != nil {
		t.Fatalf("write temp config: %v", err)
	}
	t.Setenv("RSMGO_CONFIG", path)

	cfg, err := Load()
	if err != nil {
		t.Fatalf("Load failed: %v", err)
	}

	if cfg.Addr != ":9090" {
		t.Errorf("Addr = %q, want :9090", cfg.Addr)
	}
	if cfg.EngineAddr != "127.0.0.1:50051" {
		t.Errorf("EngineAddr = %q, want 127.0.0.1:50051", cfg.EngineAddr)
	}
	if cfg.DataDir != "./share/rsmgo" {
		t.Errorf("DataDir = %q, want ./share/rsmgo", cfg.DataDir)
	}
	if !cfg.ChatStream {
		t.Errorf("ChatStream = %v, want true", cfg.ChatStream)
	}
	if len(cfg.Providers) != 1 || cfg.Providers[0] != "deepseek" {
		t.Errorf("Providers = %v, want [deepseek]", cfg.Providers)
	}
}
