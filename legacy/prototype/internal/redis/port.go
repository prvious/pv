// Package redis owns the lifecycle of the native redis binary managed by
// pv. Mirrors internal/postgres/ and internal/mysql/ but versioned:
// per-version map, version-parameterized API. State at ~/.pv/redis/{version}/
// and ~/.pv/data/redis/{version}/.
package redis

import "fmt"

func PortFor(version string) int {
	return redisPort(version)
}

func redisPort(version string) int {
	major, minor := parseVersion(version)
	return 6300 + major*100 + minor*10
}

func parseVersion(v string) (int, int) {
	var major, minor int
	fmt.Sscanf(v, "%d.%d", &major, &minor)
	return major, minor
}
