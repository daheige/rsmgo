package workspace

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"sync"
	"time"
)

// Workspace is a local directory the agent can operate in. The agent reads and
// writes directly inside Path when a session selects this workspace. Tools is
// the set of tool names the agent is allowed to use within it; an empty slice
// means no restriction (every tool is allowed).
type Workspace struct {
	ID        string    `json:"id"`
	Name      string    `json:"name"`
	Path      string    `json:"path"`
	Tools     []string  `json:"tools,omitempty"`
	CreatedAt time.Time `json:"created_at"`
}

type Store struct {
	mu  sync.RWMutex
	dir string
}

func NewStore(dir string) *Store {
	_ = os.MkdirAll(dir, 0o755)
	return &Store{dir: dir}
}

func (s *Store) path(id string) string {
	return filepath.Join(s.dir, fmt.Sprintf("%s.json", id))
}

func (s *Store) Create(w *Workspace) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	w.CreatedAt = time.Now().UTC()
	data, err := json.MarshalIndent(w, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(s.path(w.ID), data, 0o644)
}

func (s *Store) Get(id string) (*Workspace, error) {
	s.mu.RLock()
	defer s.mu.RUnlock()
	return s.read(id)
}

func (s *Store) read(id string) (*Workspace, error) {
	data, err := os.ReadFile(s.path(id))
	if err != nil {
		return nil, err
	}
	var w Workspace
	if err := json.Unmarshal(data, &w); err != nil {
		return nil, err
	}
	return &w, nil
}

func (s *Store) Delete(id string) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	return os.Remove(s.path(id))
}

func (s *Store) List() ([]*Workspace, error) {
	s.mu.RLock()
	defer s.mu.RUnlock()
	entries, err := os.ReadDir(s.dir)
	if err != nil {
		return nil, err
	}
	workspaces := make([]*Workspace, 0)
	for _, entry := range entries {
		if entry.IsDir() || filepath.Ext(entry.Name()) != ".json" {
			continue
		}
		data, err := os.ReadFile(filepath.Join(s.dir, entry.Name()))
		if err != nil {
			continue
		}
		var w Workspace
		if err := json.Unmarshal(data, &w); err != nil {
			continue
		}
		workspaces = append(workspaces, &w)
	}
	sort.SliceStable(workspaces, func(i, j int) bool {
		return workspaces[i].CreatedAt.Before(workspaces[j].CreatedAt)
	})
	return workspaces, nil
}
