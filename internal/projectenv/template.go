package projectenv

import (
	"bytes"
	"text/template"
)

// Render applies a Go text/template against vars and returns the result.
// Unknown keys (typos in pv.yml) produce an error instead of silently
// rendering "<no value>" — pv.yml is a contract, surprises are bugs.
func Render(tmplStr string, vars map[string]string) (string, error) {
	t, err := template.New("pvyml").Option("missingkey=error").Parse(tmplStr)
	if err != nil {
		return "", err
	}
	var buf bytes.Buffer
	if err := t.Execute(&buf, vars); err != nil {
		return "", err
	}
	return buf.String(), nil
}
