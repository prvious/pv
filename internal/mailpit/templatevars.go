package mailpit

// TemplateVars returns the variables available inside a pv.yml
// `mailpit.env:` block. Mailpit is single-version with fixed ports
// (SMTP 1025, HTTP 8025) — values match what the existing service
// layer publishes for the running process.
//
// Keys: smtp_host, smtp_port, http_host, http_port.
func TemplateVars() map[string]string {
	return map[string]string{
		"smtp_host": "127.0.0.1",
		"smtp_port": "1025",
		"http_host": "127.0.0.1",
		"http_port": "8025",
	}
}
