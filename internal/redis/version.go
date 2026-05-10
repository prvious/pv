package redis

import (
	"fmt"
	"os/exec"
	"regexp"
	"strings"
)

// redisVersionRE pulls the version token out of `redis-server --version`.
// Real-world output:
//
//	Redis server v=7.4.1 sha=00000000:0 malloc=libc bits=64 build=...
//
// The regexp anchors on `v=` to avoid matching version-looking
// substrings elsewhere on the line (e.g. "build=v1234"-style tokens).
var redisVersionRE = regexp.MustCompile(`v=(\d+\.\d+\.\d+)\b`)

// ProbeVersion runs `<RedisDir>/redis-server --version` and returns the
// precise version string (e.g. "7.4.1"). Used at install/update time to
// record the patch level into versions.json.
func ProbeVersion() (string, error) {
	out, err := exec.Command(ServerBinary(), "--version").Output()
	if err != nil {
		return "", fmt.Errorf("redis-server --version: %w", err)
	}
	return parseRedisVersion(string(out))
}

// parseRedisVersion is exposed (lowercase) to the test in version_test.go
// so the parser can be exercised against many real-world output lines
// without having to compile a fake redis-server for each one.
func parseRedisVersion(out string) (string, error) {
	s := strings.TrimSpace(out)
	if s == "" {
		return "", fmt.Errorf("empty redis-server --version output")
	}
	m := redisVersionRE.FindStringSubmatch(s)
	if m == nil {
		return "", fmt.Errorf("unexpected redis-server --version output: %q", s)
	}
	return m[1], nil
}
