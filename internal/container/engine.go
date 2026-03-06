package container

// CreateOpts defines parameters for creating a Docker container.
type CreateOpts struct {
	Name         string
	Image        string
	Env          []string
	Ports        map[int]int // host:container
	Volumes      map[string]string // host:container
	Labels       map[string]string
	Cmd          []string
	HealthCmd    []string
	HealthInterval string
	HealthTimeout  string
	HealthRetries  int
}

// Engine wraps Docker SDK operations.
// The actual implementation uses github.com/docker/docker/client
// and connects via the Colima Docker socket.
type Engine struct {
	socketPath string
}

func NewEngine(socketPath string) (*Engine, error) {
	return &Engine{socketPath: socketPath}, nil
}

func (e *Engine) SocketPath() string {
	return e.socketPath
}

func (e *Engine) Close() error {
	return nil
}
