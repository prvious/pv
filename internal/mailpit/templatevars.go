package mailpit

import "strconv"

// TemplateVars returns the variables available inside a pv.yml
// `mailpit.env:` block. Mailpit is single-version with fixed ports
// (SMTP and HTTP) — values come from the package's Port() /
// ConsolePort() accessors so a future port change updates one source.
//
// Keys: smtp_host, smtp_port, http_host, http_port.
func TemplateVars() map[string]string {
	return map[string]string{
		"smtp_host": "127.0.0.1",
		"smtp_port": strconv.Itoa(Port()),
		"http_host": "127.0.0.1",
		"http_port": strconv.Itoa(ConsolePort()),
	}
}
