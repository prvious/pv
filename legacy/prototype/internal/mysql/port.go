// Package mysql owns the lifecycle of native MySQL versions managed by pv.
// Mirrors internal/postgres/ — version-aware install, supervised processes,
// on-disk state at ~/.pv/mysql/<version>/ and ~/.pv/data/mysql/<version>/.
package mysql

import (
	"fmt"
	"strconv"
	"strings"
)

// PortFor returns the TCP port a mysql version should bind to.
// Scheme: 33000 + major*10 + minor.
//
//	8.0 → 33080
//	8.4 → 33084
//	9.7 → 33097
//
// version must be a "<major>.<minor>" string with major in 1..99 and
// minor in 0..99 (so the result fits comfortably below 65535 and stays
// far away from MySQL's default 3306).
func PortFor(version string) (int, error) {
	parts := strings.Split(version, ".")
	if len(parts) != 2 {
		return 0, fmt.Errorf("mysql: invalid version %q (want <major>.<minor>)", version)
	}
	major, err := strconv.Atoi(parts[0])
	if err != nil {
		return 0, fmt.Errorf("mysql: invalid major in %q: %w", version, err)
	}
	minor, err := strconv.Atoi(parts[1])
	if err != nil {
		return 0, fmt.Errorf("mysql: invalid minor in %q: %w", version, err)
	}
	if major <= 0 || major > 99 {
		return 0, fmt.Errorf("mysql: major %d out of range (1..99)", major)
	}
	if minor < 0 || minor > 99 {
		return 0, fmt.Errorf("mysql: minor %d out of range (0..99)", minor)
	}
	return 33000 + major*10 + minor, nil
}
