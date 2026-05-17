package control

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
)

const (
	ResourceMago = "mago"

	StateReady  = "ready"
	StateFailed = "failed"
)

var versionPattern = regexp.MustCompile(`^[A-Za-z0-9][A-Za-z0-9._+-]*$`)

type DesiredResource struct {
	Resource string `json:"resource"`
	Version  string `json:"version"`
}

type ObservedStatus struct {
	Resource          string `json:"resource"`
	DesiredVersion    string `json:"desired_version"`
	State             string `json:"state"`
	LastReconcileTime string `json:"last_reconcile_time"`
	LastError         string `json:"last_error,omitempty"`
	NextAction        string `json:"next_action,omitempty"`
}

type Store interface {
	PutDesired(context.Context, DesiredResource) error
	Desired(context.Context, string) (DesiredResource, bool, error)
	PutObserved(context.Context, ObservedStatus) error
	Observed(context.Context, string) (ObservedStatus, bool, error)
}

type FileStore struct {
	path string
}

func NewFileStore(path string) *FileStore {
	return &FileStore{path: path}
}

func (s *FileStore) Path() string {
	return s.path
}

func (s *FileStore) PutDesired(ctx context.Context, desired DesiredResource) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	if desired.Resource == "" {
		return errors.New("desired resource is required")
	}
	if err := ValidateVersion(desired.Version); err != nil {
		return err
	}

	snapshot, err := s.load(ctx)
	if err != nil {
		return err
	}
	snapshot.Desired[desired.Resource] = desired
	return s.save(ctx, snapshot)
}

func (s *FileStore) Desired(ctx context.Context, resource string) (DesiredResource, bool, error) {
	if err := ctx.Err(); err != nil {
		return DesiredResource{}, false, err
	}
	snapshot, err := s.load(ctx)
	if err != nil {
		return DesiredResource{}, false, err
	}
	desired, ok := snapshot.Desired[resource]
	return desired, ok, nil
}

func (s *FileStore) PutObserved(ctx context.Context, observed ObservedStatus) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	if observed.Resource == "" {
		return errors.New("observed resource is required")
	}
	if observed.State == "" {
		return errors.New("observed state is required")
	}

	snapshot, err := s.load(ctx)
	if err != nil {
		return err
	}
	snapshot.Observed[observed.Resource] = observed
	return s.save(ctx, snapshot)
}

func (s *FileStore) Observed(ctx context.Context, resource string) (ObservedStatus, bool, error) {
	if err := ctx.Err(); err != nil {
		return ObservedStatus{}, false, err
	}
	snapshot, err := s.load(ctx)
	if err != nil {
		return ObservedStatus{}, false, err
	}
	observed, ok := snapshot.Observed[resource]
	return observed, ok, nil
}

func ValidateVersion(version string) error {
	if !versionPattern.MatchString(version) {
		return fmt.Errorf("invalid version %q", version)
	}
	return nil
}

type snapshot struct {
	Desired  map[string]DesiredResource `json:"desired"`
	Observed map[string]ObservedStatus  `json:"observed"`
}

func newSnapshot() snapshot {
	return snapshot{
		Desired:  make(map[string]DesiredResource),
		Observed: make(map[string]ObservedStatus),
	}
}

func (s *FileStore) load(ctx context.Context) (snapshot, error) {
	if err := ctx.Err(); err != nil {
		return snapshot{}, err
	}

	data, err := os.ReadFile(s.path)
	if errors.Is(err, os.ErrNotExist) {
		return newSnapshot(), nil
	}
	if err != nil {
		return snapshot{}, err
	}

	current := newSnapshot()
	if err := json.Unmarshal(data, &current); err != nil {
		return snapshot{}, err
	}
	if current.Desired == nil {
		current.Desired = make(map[string]DesiredResource)
	}
	if current.Observed == nil {
		current.Observed = make(map[string]ObservedStatus)
	}
	return current, nil
}

func (s *FileStore) save(ctx context.Context, current snapshot) error {
	if err := ctx.Err(); err != nil {
		return err
	}

	if err := os.MkdirAll(filepath.Dir(s.path), 0o755); err != nil {
		return err
	}

	data, err := json.MarshalIndent(current, "", "  ")
	if err != nil {
		return err
	}
	data = append(data, '\n')

	temp, err := os.CreateTemp(filepath.Dir(s.path), ".pv-state-*")
	if err != nil {
		return err
	}
	tempPath := temp.Name()
	defer os.Remove(tempPath)

	if _, err := temp.Write(data); err != nil {
		temp.Close()
		return err
	}
	if err := temp.Chmod(0o600); err != nil {
		temp.Close()
		return err
	}
	if err := temp.Close(); err != nil {
		return err
	}
	return os.Rename(tempPath, s.path)
}
