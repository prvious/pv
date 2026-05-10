package postgres

import (
	"fmt"
	"strconv"
)

// TemplateVars returns the variables available inside a pv.yml
// `postgresql.env:` block. The caller passes the major (e.g., "18")
// from pv.yml and the probed fullVersion (e.g., "18.1"); both come
// from outside so this function stays pure and testable.
func TemplateVars(major, fullVersion string) (map[string]string, error) {
	port, err := PortFor(major)
	if err != nil {
		return nil, err
	}
	const user = "postgres"
	const pass = "postgres"
	const host = "127.0.0.1"
	return map[string]string{
		"host":     host,
		"port":     strconv.Itoa(port),
		"username": user,
		"password": pass,
		"version":  fullVersion,
		"dsn":      fmt.Sprintf("postgresql://%s:%s@%s:%d", user, pass, host, port),
	}, nil
}
