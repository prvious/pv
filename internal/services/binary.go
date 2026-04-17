package services

import (
	"time"

	"github.com/prvious/pv/internal/binaries"
)

// ReadyCheck describes how a supervisor verifies that a binary service has
// finished starting and is ready to accept requests. Construct via TCPReady
// or HTTPReady — the unexported fields prevent constructing invalid states
// (zero-value or both-set) from outside this package.
type ReadyCheck struct {
	tcpPort      int           // probe 127.0.0.1:port until Dial succeeds
	httpEndpoint string        // GET this URL, expect a 2xx response
	Timeout      time.Duration // overall give-up time for the ready probe
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

// BinaryService is the contract for services that run as native binaries
// supervised by the pv daemon, rather than as Docker containers.
type BinaryService interface {
	Name() string
	DisplayName() string

	// Binary returns the binaries.Binary descriptor that owns download / URL logic.
	Binary() binaries.Binary

	// Args returns CLI args passed to the binary at spawn time.
	// dataDir is the absolute path to this service's persistent data directory.
	Args(dataDir string) []string

	// Env returns process env vars to add on top of os.Environ().
	Env() []string

	// Port is the primary service port exposed on 127.0.0.1.
	Port() int

	// ConsolePort is a secondary port (admin UI), or 0 if none.
	ConsolePort() int

	// WebRoutes exposes HTTP subdomains (e.g. s3.pv.test -> 9001) to FrankenPHP.
	WebRoutes() []WebRoute

	// EnvVars returns the env vars injected into a linked project's .env file.
	EnvVars(projectName string) map[string]string

	// ReadyCheck describes how to verify the spawned process is accepting requests.
	ReadyCheck() ReadyCheck
}

// binaryRegistry is populated by init() functions in per-service files
// (e.g. rustfs.go registers itself as "s3").
var binaryRegistry = map[string]BinaryService{}

// LookupBinary returns the BinaryService registered under name, or ok=false.
func LookupBinary(name string) (BinaryService, bool) {
	svc, ok := binaryRegistry[name]
	return svc, ok
}

// AllBinary returns a snapshot of the binary-service registry.
// Callers must not mutate the returned map.
func AllBinary() map[string]BinaryService {
	out := make(map[string]BinaryService, len(binaryRegistry))
	for k, v := range binaryRegistry {
		out[k] = v
	}
	return out
}
