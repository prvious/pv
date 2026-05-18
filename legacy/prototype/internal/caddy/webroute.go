package caddy

// WebRoute maps a subdomain under pv.{tld} to a local port.
// For example, {Subdomain: "s3", Port: 9001} routes s3.pv.test → 127.0.0.1:9001.
type WebRoute struct {
	Subdomain string
	Port      int
}
