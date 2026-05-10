package supervisor

import (
	"context"
	"fmt"
	"net"
	"net/http"
	"time"
)

// ReadyCheck describes how a supervisor verifies that a binary service has
// finished starting and is ready to accept requests. Construct via TCPReady
// or HTTPReady — the unexported fields prevent constructing invalid states
// (zero-value or both-set) from outside this package.
type ReadyCheck struct {
	tcpPort      int
	httpEndpoint string
	Timeout      time.Duration
}

// TCPReady returns a ReadyCheck that probes 127.0.0.1:port via TCP Dial.
func TCPReady(port int, timeout time.Duration) ReadyCheck {
	return ReadyCheck{tcpPort: port, Timeout: timeout}
}

// HTTPReady returns a ReadyCheck that GETs the given URL and expects a 2xx.
func HTTPReady(url string, timeout time.Duration) ReadyCheck {
	return ReadyCheck{httpEndpoint: url, Timeout: timeout}
}

// TCPPort returns the TCP probe port, or 0 if this is an HTTP check.
func (r ReadyCheck) TCPPort() int { return r.tcpPort }

// HTTPEndpoint returns the HTTP probe URL, or "" if this is a TCP check.
func (r ReadyCheck) HTTPEndpoint() string { return r.httpEndpoint }

// BuildReadyFunc returns a func(ctx) error appropriate to the ReadyCheck variant.
// The ReadyCheck must specify exactly one of TCPPort or HTTPEndpoint.
func BuildReadyFunc(rc ReadyCheck) (func(context.Context) error, error) {
	httpSet := rc.HTTPEndpoint() != ""
	tcpSet := rc.TCPPort() > 0
	switch {
	case httpSet && tcpSet:
		return nil, fmt.Errorf("invalid ReadyCheck: both TCPPort and HTTPEndpoint set; specify exactly one")
	case httpSet:
		client := &http.Client{Timeout: 2 * time.Second}
		url := rc.HTTPEndpoint()
		return func(ctx context.Context) error {
			req, err := http.NewRequestWithContext(ctx, http.MethodGet, url, nil)
			if err != nil {
				return err
			}
			resp, err := client.Do(req)
			if err != nil {
				return err
			}
			defer resp.Body.Close()
			if resp.StatusCode >= 200 && resp.StatusCode < 300 {
				return nil
			}
			return fmt.Errorf("HTTP %s returned %d", url, resp.StatusCode)
		}, nil
	case tcpSet:
		addr := fmt.Sprintf("127.0.0.1:%d", rc.TCPPort())
		return func(ctx context.Context) error {
			d := net.Dialer{Timeout: 500 * time.Millisecond}
			c, err := d.DialContext(ctx, "tcp", addr)
			if err != nil {
				return err
			}
			c.Close()
			return nil
		}, nil
	default:
		return nil, fmt.Errorf("invalid ReadyCheck: must set exactly one of TCPPort or HTTPEndpoint")
	}
}
