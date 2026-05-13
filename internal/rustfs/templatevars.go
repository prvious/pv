package rustfs

import (
	"fmt"
)

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
