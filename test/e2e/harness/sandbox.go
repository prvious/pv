package harness

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"hash/fnv"
	"io/fs"
	"maps"
	"net"
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"strings"
	"time"
)

type Sandbox struct {
	RootDir     string
	HomeDir     string
	PVRoot      string
	StateDir    string
	CacheDir    string
	ConfigDir   string
	DataDir     string
	LogsDir     string
	ProjectRoot string
	TempDir     string

	nextPort     int
	reservations []*PortReservation
	commands     []*exec.Cmd
	cleaned      bool
}

type sandboxConfig struct {
	RootDir     string
	HomeDir     string
	ProjectRoot string
}

type PortReservation struct {
	Port     int
	listener net.Listener
	closed   bool
}

type CommandRunner struct {
	Executable string
	Sandbox    *Sandbox
	ExtraEnv   map[string]string
}

type CommandResult struct {
	Argv             []string
	WorkingDirectory string
	EnvDiff          map[string]string
	Stdout           string
	Stderr           string
	ExitCode         int
	Elapsed          time.Duration
	LogPaths         []string
	Err              error
}

func NewSandbox(rootDir string) (*Sandbox, error) {
	return newSandbox(sandboxConfig{RootDir: rootDir})
}

func newSandbox(config sandboxConfig) (*Sandbox, error) {
	rootDir, err := cleanAbs(config.RootDir, "sandbox root")
	if err != nil {
		return nil, err
	}

	homeDir := config.HomeDir
	if homeDir == "" {
		homeDir = filepath.Join(rootDir, "home")
	}
	homeDir, err = cleanAbs(homeDir, "sandbox home")
	if err != nil {
		return nil, err
	}

	projectRoot := config.ProjectRoot
	if projectRoot == "" {
		projectRoot = filepath.Join(rootDir, "project")
	}
	projectRoot, err = cleanAbs(projectRoot, "sandbox project root")
	if err != nil {
		return nil, err
	}

	pvRoot := filepath.Join(homeDir, ".pv")
	sandbox := &Sandbox{
		RootDir:     rootDir,
		HomeDir:     homeDir,
		PVRoot:      pvRoot,
		StateDir:    filepath.Join(pvRoot, "state"),
		CacheDir:    filepath.Join(pvRoot, "cache"),
		ConfigDir:   filepath.Join(pvRoot, "config"),
		DataDir:     filepath.Join(pvRoot, "data"),
		LogsDir:     filepath.Join(pvRoot, "logs"),
		ProjectRoot: projectRoot,
		TempDir:     filepath.Join(rootDir, "tmp"),
		nextPort:    deterministicPortBase(rootDir),
	}
	if err := sandbox.validateSafePaths(); err != nil {
		return nil, err
	}
	for _, dir := range []string{
		sandbox.HomeDir,
		sandbox.StateDir,
		sandbox.CacheDir,
		sandbox.ConfigDir,
		sandbox.DataDir,
		sandbox.LogsDir,
		sandbox.ProjectRoot,
		sandbox.TempDir,
	} {
		if err := os.MkdirAll(dir, 0o755); err != nil {
			return nil, fmt.Errorf("create sandbox directory %s: %w", dir, err)
		}
	}

	return sandbox, nil
}

func (s *Sandbox) ReservePort() (*PortReservation, error) {
	const attempts = 1000
	for range attempts {
		port := s.nextCandidatePort()
		listener, err := net.Listen("tcp", fmt.Sprintf("127.0.0.1:%d", port))
		if err != nil {
			continue
		}
		reservation := &PortReservation{Port: port, listener: listener}
		s.reservations = append(s.reservations, reservation)
		return reservation, nil
	}
	return nil, errors.New("reserve sandbox port: no available candidate ports")
}

func (s *Sandbox) TrackCommand(command *exec.Cmd) {
	if command != nil {
		s.commands = append(s.commands, command)
	}
}

func (s *Sandbox) Cleanup() error {
	if s == nil || s.cleaned {
		return nil
	}
	s.cleaned = true

	var errs []error
	for _, command := range s.commands {
		if command.Process == nil || command.ProcessState != nil {
			continue
		}
		if err := command.Process.Kill(); err != nil && !errors.Is(err, os.ErrProcessDone) {
			errs = append(errs, fmt.Errorf("kill process %d: %w", command.Process.Pid, err))
		}
		if err := command.Wait(); err != nil {
			var exitErr *exec.ExitError
			if !errors.As(err, &exitErr) {
				errs = append(errs, fmt.Errorf("wait for process %d: %w", command.Process.Pid, err))
			}
		}
	}
	for _, reservation := range s.reservations {
		if err := reservation.Close(); err != nil && !errors.Is(err, net.ErrClosed) {
			errs = append(errs, fmt.Errorf("close port reservation %d: %w", reservation.Port, err))
		}
	}
	if err := os.RemoveAll(s.RootDir); err != nil {
		errs = append(errs, fmt.Errorf("remove sandbox root %s: %w", s.RootDir, err))
	}
	return errors.Join(errs...)
}

func (s *Sandbox) Env() map[string]string {
	return map[string]string{
		"HOME":                s.HomeDir,
		"PV_E2E_CACHE_DIR":    s.CacheDir,
		"PV_E2E_CONFIG_DIR":   s.ConfigDir,
		"PV_E2E_DATA_DIR":     s.DataDir,
		"PV_E2E_LOG_DIR":      s.LogsDir,
		"PV_E2E_PROJECT_ROOT": s.ProjectRoot,
		"PV_E2E_ROOT":         s.RootDir,
		"PV_E2E_STATE_DIR":    s.StateDir,
		"PV_E2E_TEMP_DIR":     s.TempDir,
		"PV_E2E_PV_ROOT":      s.PVRoot,
		"TMPDIR":              s.TempDir,
		"XDG_CACHE_HOME":      s.CacheDir,
		"XDG_CONFIG_HOME":     s.ConfigDir,
	}
}

func (s *Sandbox) LogPaths() ([]string, error) {
	var paths []string
	err := filepath.WalkDir(s.LogsDir, func(path string, entry fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if entry.Type().IsRegular() {
			paths = append(paths, path)
		}
		return nil
	})
	if err != nil {
		return nil, err
	}
	slices.Sort(paths)
	return paths, nil
}

func (r CommandRunner) Run(ctx context.Context, workDir string, args ...string) CommandResult {
	argv := append([]string{r.Executable}, args...)
	command := exec.CommandContext(ctx, r.Executable, args...)
	command.Dir = workDir

	parentEnv := envMap(os.Environ())
	overrides := map[string]string{}
	if r.Sandbox != nil {
		maps.Copy(overrides, r.Sandbox.Env())
	}
	maps.Copy(overrides, r.ExtraEnv)
	mergedEnv := maps.Clone(parentEnv)
	maps.Copy(mergedEnv, overrides)
	command.Env = flattenEnv(mergedEnv)

	var stdout bytes.Buffer
	var stderr bytes.Buffer
	command.Stdout = &stdout
	command.Stderr = &stderr

	started := time.Now()
	err := command.Run()
	logPaths, logErr := r.logPaths()
	if logErr != nil {
		err = errors.Join(err, logErr)
	}

	return CommandResult{
		Argv:             argv,
		WorkingDirectory: workDir,
		EnvDiff:          diffEnv(parentEnv, mergedEnv),
		Stdout:           stdout.String(),
		Stderr:           stderr.String(),
		ExitCode:         exitCode(err),
		Elapsed:          time.Since(started),
		LogPaths:         logPaths,
		Err:              err,
	}
}

func (r CommandRunner) logPaths() ([]string, error) {
	if r.Sandbox == nil {
		return nil, nil
	}
	return r.Sandbox.LogPaths()
}

func (p *PortReservation) Close() error {
	if p == nil || p.closed {
		return nil
	}
	p.closed = true
	return p.listener.Close()
}

func (s *Sandbox) nextCandidatePort() int {
	port := s.nextPort
	s.nextPort++
	if s.nextPort > 59999 {
		s.nextPort = 20000
	}
	return port
}

func (s *Sandbox) validateSafePaths() error {
	realHome, err := os.UserHomeDir()
	if err != nil {
		return fmt.Errorf("resolve user home for sandbox safety: %w", err)
	}
	realPVRoot := filepath.Join(realHome, ".pv")
	if samePath(s.HomeDir, realHome) || samePath(s.PVRoot, realPVRoot) {
		return fmt.Errorf("sandbox would use real ~/.pv at %s", realPVRoot)
	}
	for _, path := range []string{s.HomeDir, s.PVRoot, s.ProjectRoot, s.TempDir} {
		if !isPathWithin(path, s.RootDir) {
			return fmt.Errorf("sandbox path %s is outside root %s", path, s.RootDir)
		}
	}
	return nil
}

func deterministicPortBase(seed string) int {
	hash := fnv.New32a()
	_, _ = hash.Write([]byte(seed))
	return 20000 + int(hash.Sum32()%20000)
}

func cleanAbs(path string, label string) (string, error) {
	if path == "" {
		return "", fmt.Errorf("%s is required", label)
	}
	absolute, err := filepath.Abs(path)
	if err != nil {
		return "", fmt.Errorf("resolve %s: %w", label, err)
	}
	return filepath.Clean(absolute), nil
}

func samePath(left string, right string) bool {
	leftAbs, err := filepath.Abs(left)
	if err != nil {
		return false
	}
	rightAbs, err := filepath.Abs(right)
	if err != nil {
		return false
	}
	return filepath.Clean(leftAbs) == filepath.Clean(rightAbs)
}

func isPathWithin(path string, root string) bool {
	rel, err := filepath.Rel(root, path)
	if err != nil {
		return false
	}
	return rel == "." || (rel != ".." && !strings.HasPrefix(rel, ".."+string(os.PathSeparator)))
}

func envMap(env []string) map[string]string {
	result := make(map[string]string, len(env))
	for _, entry := range env {
		key, value, ok := strings.Cut(entry, "=")
		if ok {
			result[key] = value
		}
	}
	return result
}

func diffEnv(parent map[string]string, merged map[string]string) map[string]string {
	diff := map[string]string{}
	for key, value := range merged {
		parentValue, ok := parent[key]
		if !ok || parentValue != value {
			diff[key] = value
		}
	}
	return diff
}

func flattenEnv(env map[string]string) []string {
	keys := slices.Sorted(maps.Keys(env))
	result := make([]string, 0, len(keys))
	for _, key := range keys {
		result = append(result, key+"="+env[key])
	}
	return result
}
