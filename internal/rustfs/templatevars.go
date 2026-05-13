package rustfs

import (
	"fmt"
)

// TemplateVars returns the variables available inside a pv.yml
// `rustfs.env:` block. RustFS is single-version with fixed ports
// (console and API) — values come from the package's Port() /
// ConsolePort() accessors so a future port change updates one source.
//
// Keys: endpoint, access_key, secret_key, region, use_path_style.
func TemplateVars(version string) map[string]string {
	if err := ValidateVersion(version); err != nil {
		return map[string]string{}
	}
	return map[string]string{
		"endpoint":       fmt.Sprintf("http://127.0.0.1:%d", Port()),
		"access_key":     "rstfsadmin",
		"secret_key":     "rstfsadmin",
		"region":         "us-east-1",
		"use_path_style": "true",
	}
}
