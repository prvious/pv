package rustfs

import (
	"fmt"
)

// TemplateVars returns the variables available inside a pv.yml
// `rustfs.env:` block. Values mirror the existing service.EnvVars
// defaults (admin/admin credentials, us-east-1, path-style addressing)
// minus the project-name-derived bucket — bucket creation is now an
// explicit user command, not a pv.yml side effect. The endpoint port
// comes from the package's Port() accessor so a future port change
// updates one source.
//
// Keys: endpoint, access_key, secret_key, region, use_path_style.
func TemplateVars() map[string]string {
	return map[string]string{
		"endpoint":       fmt.Sprintf("http://127.0.0.1:%d", Port()),
		"access_key":     "rstfsadmin",
		"secret_key":     "rstfsadmin",
		"region":         "us-east-1",
		"use_path_style": "true",
	}
}
