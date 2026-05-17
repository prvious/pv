package host

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"strings"
)

var segmentPattern = regexp.MustCompile(`^[A-Za-z0-9][A-Za-z0-9._+-]*$`)

type Paths struct {
	root string
}

func NewPaths() (Paths, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return Paths{}, err
	}
	return NewPathsFromHome(home)
}

func NewPathsFromHome(home string) (Paths, error) {
	if home == "" {
		return Paths{}, errors.New("home directory is required")
	}
	return NewPathsFromRoot(filepath.Join(home, ".pv"))
}

func NewPathsFromRoot(root string) (Paths, error) {
	if root == "" {
		return Paths{}, errors.New("pv root is required")
	}
	return Paths{root: filepath.Clean(root)}, nil
}

func (p Paths) Root() string {
	return p.root
}

func (p Paths) BinDir() string {
	return filepath.Join(p.root, "bin")
}

func (p Paths) PHPRuntimeDir(version string) (string, error) {
	if err := validateSegment("version", version); err != nil {
		return "", err
	}
	return filepath.Join(p.root, "runtimes", "php", version), nil
}

func (p Paths) ToolDir(name string, version string) (string, error) {
	if err := validateSegment("tool name", name); err != nil {
		return "", err
	}
	if err := validateSegment("version", version); err != nil {
		return "", err
	}
	return filepath.Join(p.root, "tools", name, version), nil
}

func (p Paths) ServiceBinDir(name string, version string) (string, error) {
	if err := validateSegment("service name", name); err != nil {
		return "", err
	}
	if err := validateSegment("version", version); err != nil {
		return "", err
	}
	return filepath.Join(p.root, "services", name, version, "bin"), nil
}

func (p Paths) DataDir(name string, version string) (string, error) {
	if err := validateSegment("resource name", name); err != nil {
		return "", err
	}
	if err := validateSegment("version", version); err != nil {
		return "", err
	}
	return filepath.Join(p.root, "data", name, version), nil
}

func (p Paths) LogPath(name string, version string) (string, error) {
	if err := validateSegment("resource name", name); err != nil {
		return "", err
	}
	if err := validateSegment("version", version); err != nil {
		return "", err
	}
	return filepath.Join(p.root, "logs", name, version+".log"), nil
}

func (p Paths) StateDBPath() string {
	return filepath.Join(p.root, "state", "pv.db")
}

func (p Paths) CacheArtifactsDir() string {
	return filepath.Join(p.root, "cache", "artifacts")
}

func (p Paths) ConfigDir() string {
	return filepath.Join(p.root, "config")
}

func (p Paths) ValidateManagedPath(path string) error {
	clean := filepath.Clean(path)
	if clean == p.BinDir() {
		return errors.New("bin directory is reserved for shims only")
	}
	if !isWithin(clean, p.root) {
		return fmt.Errorf("path %q is outside pv root", path)
	}
	dataRoot := filepath.Join(p.root, "data")
	separator := string(filepath.Separator)
	hasDataSegment := strings.Contains(clean, separator+"data"+separator) || strings.HasSuffix(clean, separator+"data")
	if isWithin(clean, filepath.Join(p.root, "services")) && hasDataSegment {
		return fmt.Errorf("data path %q must use %s", path, dataRoot)
	}
	return nil
}

func validateSegment(label string, value string) error {
	if !segmentPattern.MatchString(value) {
		return fmt.Errorf("invalid %s %q", label, value)
	}
	return nil
}

func isWithin(path string, root string) bool {
	rel, err := filepath.Rel(root, path)
	if err != nil {
		return false
	}
	return rel == "." || (rel != ".." && !strings.HasPrefix(rel, ".."+string(filepath.Separator)))
}
