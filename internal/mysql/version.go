package mysql

import (
	"fmt"
	"os/exec"
	"path/filepath"
	"regexp"
	"strings"

	"github.com/prvious/pv/internal/config"
)

// mysqldVersionRE pulls the patch-level version string out of
// `mysqld --version` output. Real-world examples:
//
//	mysqld  Ver 8.4.9 for macos15 on arm64 (MySQL Community Server - GPL)
//	mysqld  Ver 9.7.0 for macos15 on arm64 (MySQL Community Server - GPL)
//
// The first whitespace run after "mysqld" is sometimes a tab on Homebrew
// builds — `\s+` handles both. The regexp anchors on " Ver " to avoid
// matching version-looking substrings elsewhere in the line.
var mysqldVersionRE = regexp.MustCompile(`Ver\s+(\d+\.\d+\.\d+)\b`)

// ProbeVersion runs `<bin>/mysqld --version` and returns the precise
// version string (e.g. "8.4.9"). The version argument selects the install
// root; the answer is the patch within that major.minor.
func ProbeVersion(version string) (string, error) {
	binPath := filepath.Join(config.MysqlBinDir(version), "mysqld")
	out, err := exec.Command(binPath, "--version").Output()
	if err != nil {
		return "", fmt.Errorf("mysqld --version: %w", err)
	}
	return parseMysqldVersion(string(out))
}

// parseMysqldVersion is exposed (lowercase) to the test in version_test.go
// so the parser can be exercised against many real-world output lines
// without having to compile a fake mysqld for each one.
func parseMysqldVersion(out string) (string, error) {
	s := strings.TrimSpace(out)
	if s == "" {
		return "", fmt.Errorf("empty mysqld --version output")
	}
	m := mysqldVersionRE.FindStringSubmatch(s)
	if m == nil {
		return "", fmt.Errorf("unexpected mysqld --version output: %q", s)
	}
	return m[1], nil
}
