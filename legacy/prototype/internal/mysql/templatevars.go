package mysql

import (
	"fmt"
	"strconv"
)

// TemplateVars returns the variables available inside a pv.yml
// `mysql.env:` block. The caller passes the version (e.g., "8.0")
// from pv.yml and the probed fullVersion (e.g., "8.0.36"); both come
// from outside so this function stays pure and testable.
//
// Keys: host, port, username, password, version, dsn.
func TemplateVars(version, fullVersion string) (map[string]string, error) {
	port, err := PortFor(version)
	if err != nil {
		return nil, err
	}
	const user = "root"
	const pass = ""
	const host = "127.0.0.1"
	return map[string]string{
		"host":     host,
		"port":     strconv.Itoa(port),
		"username": user,
		"password": pass,
		"version":  fullVersion,
		"dsn":      fmt.Sprintf("mysql://%s:%s@%s:%d", user, pass, host, port),
	}, nil
}
