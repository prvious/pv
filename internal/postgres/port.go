package postgres

import (
	"fmt"
	"strconv"
)

// PortFor returns the TCP port a postgres major should bind to.
// Scheme: 54000 + major. Major must be a numeric string ("17", "18", …).
func PortFor(major string) (int, error) {
	n, err := strconv.Atoi(major)
	if err != nil {
		return 0, fmt.Errorf("postgres: invalid major %q: %w", major, err)
	}
	if n <= 0 {
		return 0, fmt.Errorf("postgres: invalid major %q (non-positive)", major)
	}
	return 54000 + n, nil
}
