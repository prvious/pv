package postgres

import (
	"fmt"
	"strconv"
)

// PortFor returns the TCP port a postgres major should bind to.
// Scheme: 54000 + major. Major must be a numeric string in 1..999;
// real postgres majors are tiny (17, 18, …), and the bound prevents
// PortFor("99999") from silently returning 153999 (out of TCP range).
func PortFor(major string) (int, error) {
	n, err := strconv.Atoi(major)
	if err != nil {
		return 0, fmt.Errorf("postgres: invalid major %q: %w", major, err)
	}
	if n <= 0 || n > 999 {
		return 0, fmt.Errorf("postgres: major %q out of range (1..999)", major)
	}
	return 54000 + n, nil
}
