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
	CurrentSchemaVersion = 1

	ResourceComposer = "composer"
	ResourceMailpit  = "mailpit"
	ResourceMago     = "mago"
	ResourceMySQL    = "mysql"
	ResourcePostgres = "postgres"
	ResourcePHP      = "php"
	ResourceRedis    = "redis"
	ResourceRustFS   = "rustfs"

	StateBlocked = "blocked"
	StateMissing = "missing"
	StateReady   = "ready"
	StateStopped = "stopped"
	StateFailed  = "failed"
)

var versionPattern = regexp.MustCompile(`^[A-Za-z0-9][A-Za-z0-9._+-]*$`)

type DesiredResource struct {
	Resource       string `json:"resource"`
	Version        string `json:"version"`
	RuntimeVersion string `json:"runtime_version,omitempty"`
}

type ObservedStatus struct {
	Resource          string `json:"resource"`
	DesiredVersion    string `json:"desired_version"`
	RuntimeVersion    string `json:"runtime_version,omitempty"`
	State             string `json:"state"`
	LastReconcileTime string `json:"last_reconcile_time"`
	LastError         string `json:"last_error,omitempty"`
	NextAction        string `json:"next_action,omitempty"`
}

type Store interface {
	Migrate(context.Context) error
	SchemaVersion(context.Context) (int, error)
	PutDesired(context.Context, DesiredResource) error
	Desired(context.Context, string) (DesiredResource, bool, error)
	PutObserved(context.Context, ObservedStatus) error
	Observed(context.Context, string) (ObservedStatus, bool, error)
}

type FileStore struct {
	path   string
	runner MigrationRunner
}

func NewFileStore(path string) *FileStore {
	return &FileStore{
		path:   path,
		runner: NewMigrationRunner(defaultMigrations()),
	}
}

func NewFileStoreWithRunner(path string, runner MigrationRunner) *FileStore {
	if runner == nil {
		runner = NewMigrationRunner(defaultMigrations())
	}
	return &FileStore{path: path, runner: runner}
}

func (s *FileStore) Path() string {
	return s.path
}

func (s *FileStore) Migrate(ctx context.Context) error {
	current, migrated, err := s.loadMigrated(ctx)
	if err != nil {
		return err
	}
	if !migrated {
		return nil
	}
	return s.save(ctx, current)
}

func (s *FileStore) SchemaVersion(ctx context.Context) (int, error) {
	current, _, err := s.loadMigrated(ctx)
	if err != nil {
		return 0, err
	}
	return current.SchemaVersion, nil
}

func (s *FileStore) PutDesired(ctx context.Context, desired DesiredResource) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	if err := validateDesiredResource(desired); err != nil {
		return err
	}

	snapshot, _, err := s.loadMigrated(ctx)
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
	snapshot, _, err := s.loadMigrated(ctx)
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

	snapshot, _, err := s.loadMigrated(ctx)
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
	snapshot, _, err := s.loadMigrated(ctx)
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

func validateDesiredResource(desired DesiredResource) error {
	if desired.Resource == "" {
		return errors.New("desired resource is required")
	}
	if err := ValidateVersion(desired.Version); err != nil {
		return err
	}
	if desired.RuntimeVersion == "" {
		return nil
	}
	return ValidateVersion(desired.RuntimeVersion)
}

type AppliedMigration struct {
	ID string `json:"id"`
}

type StoreSnapshot struct {
	SchemaVersion     int                        `json:"schema_version"`
	AppliedMigrations []AppliedMigration         `json:"applied_migrations"`
	Desired           map[string]DesiredResource `json:"desired"`
	Observed          map[string]ObservedStatus  `json:"observed"`
}

func newSnapshot() StoreSnapshot {
	return StoreSnapshot{
		Desired:  make(map[string]DesiredResource),
		Observed: make(map[string]ObservedStatus),
	}
}

type Migration struct {
	ID    string
	Apply func(context.Context, StoreSnapshot) (StoreSnapshot, error)
}

type MigrationRunner interface {
	Run(context.Context, StoreSnapshot) (StoreSnapshot, bool, error)
}

type ForwardMigrationRunner struct {
	migrations []Migration
}

func NewMigrationRunner(migrations []Migration) ForwardMigrationRunner {
	return ForwardMigrationRunner{migrations: migrations}
}

func (r ForwardMigrationRunner) Run(ctx context.Context, current StoreSnapshot) (StoreSnapshot, bool, error) {
	if current.SchemaVersion > CurrentSchemaVersion {
		return StoreSnapshot{}, false, fmt.Errorf("store schema version %d is newer than supported version %d", current.SchemaVersion, CurrentSchemaVersion)
	}

	applied := make(map[string]bool, len(current.AppliedMigrations))
	for _, migration := range current.AppliedMigrations {
		applied[migration.ID] = true
	}

	changed := false
	for _, migration := range r.migrations {
		if err := ctx.Err(); err != nil {
			return StoreSnapshot{}, false, err
		}
		if applied[migration.ID] {
			continue
		}
		next, err := migration.Apply(ctx, current)
		if err != nil {
			return StoreSnapshot{}, false, fmt.Errorf("apply migration %s: %w", migration.ID, err)
		}
		current = next
		current.AppliedMigrations = append(current.AppliedMigrations, AppliedMigration{ID: migration.ID})
		changed = true
	}
	return current, changed, nil
}

func defaultMigrations() []Migration {
	return []Migration{
		{
			ID: "0001_initial_json_store",
			Apply: func(ctx context.Context, current StoreSnapshot) (StoreSnapshot, error) {
				if err := ctx.Err(); err != nil {
					return StoreSnapshot{}, err
				}
				current.SchemaVersion = CurrentSchemaVersion
				if current.Desired == nil {
					current.Desired = make(map[string]DesiredResource)
				}
				if current.Observed == nil {
					current.Observed = make(map[string]ObservedStatus)
				}
				return current, nil
			},
		},
	}
}

func (s *FileStore) loadMigrated(ctx context.Context) (StoreSnapshot, bool, error) {
	current, err := s.load(ctx)
	if err != nil {
		return StoreSnapshot{}, false, err
	}
	migrated, changed, err := s.runner.Run(ctx, current)
	if err != nil {
		return StoreSnapshot{}, false, err
	}
	return migrated, changed, nil
}

func (s *FileStore) load(ctx context.Context) (StoreSnapshot, error) {
	if err := ctx.Err(); err != nil {
		return StoreSnapshot{}, err
	}

	data, err := os.ReadFile(s.path)
	if errors.Is(err, os.ErrNotExist) {
		return newSnapshot(), nil
	}
	if err != nil {
		return StoreSnapshot{}, err
	}

	current := newSnapshot()
	if err := json.Unmarshal(data, &current); err != nil {
		return StoreSnapshot{}, fmt.Errorf("load store state %s: %w", s.path, err)
	}
	if current.Desired == nil {
		current.Desired = make(map[string]DesiredResource)
	}
	if current.Observed == nil {
		current.Observed = make(map[string]ObservedStatus)
	}
	return current, nil
}

func (s *FileStore) save(ctx context.Context, current StoreSnapshot) error {
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
