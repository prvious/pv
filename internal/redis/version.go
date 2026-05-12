package redis

import (
	"fmt"
	"os/exec"
	"regexp"
	"strings"
)

var redisVersionRE = regexp.MustCompile(`v=(\d+\.\d+\.\d+)\b`)

func ProbeVersion(version string) (string, error) {
	binPath := ServerBinary(version)
	out, err := exec.Command(binPath, "--version").Output()
	if err != nil {
		return "", fmt.Errorf("redis-server --version: %w", err)
	}
	return parseRedisVersion(string(out))
}

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
