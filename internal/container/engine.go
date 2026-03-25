package container

import (
	"context"
	"fmt"
	"io"
	"strconv"
	"time"

	"github.com/docker/docker/api/types/container"
	"github.com/docker/docker/api/types/image"
	"github.com/docker/docker/api/types/mount"
	"github.com/docker/docker/api/types/network"
	"github.com/docker/docker/client"
	"github.com/docker/docker/pkg/stdcopy"
	"github.com/docker/go-connections/nat"
)

// CreateOpts defines parameters for creating a Docker container.
type CreateOpts struct {
	Name           string
	Image          string
	Env            []string
	Ports          map[int]int       // host:container
	Volumes        map[string]string // host:container
	Labels         map[string]string
	Cmd            []string
	HealthCmd      []string
	HealthInterval string
	HealthTimeout  string
	HealthRetries  int
}

// Engine wraps Docker SDK operations.
// The actual implementation uses github.com/docker/docker/client
// and connects via the Colima Docker socket.
type Engine struct {
	socketPath string
	client     *client.Client
}

// NewEngine creates a Docker client connected via the given Unix socket path.
func NewEngine(socketPath string) (*Engine, error) {
	cli, err := client.NewClientWithOpts(
		client.WithHost("unix://"+socketPath),
		client.WithAPIVersionNegotiation(),
	)
	if err != nil {
		return nil, fmt.Errorf("docker client: %w", err)
	}
	return &Engine{socketPath: socketPath, client: cli}, nil
}

// SocketPath returns the Docker socket path this engine is connected to.
func (e *Engine) SocketPath() string {
	return e.socketPath
}

// Close closes the Docker client connection.
func (e *Engine) Close() error {
	return e.client.Close()
}

// Pull pulls a Docker image by name, draining the response body.
func (e *Engine) Pull(ctx context.Context, imageName string) error {
	reader, err := e.client.ImagePull(ctx, imageName, image.PullOptions{})
	if err != nil {
		return fmt.Errorf("pull %s: %w", imageName, err)
	}
	defer reader.Close()
	// Drain output to complete the pull.
	if _, err := io.Copy(io.Discard, reader); err != nil {
		return fmt.Errorf("pull %s: reading response: %w", imageName, err)
	}
	return nil
}

// CreateAndStart creates and starts a container from the given options.
// It removes any existing container with the same name first (idempotent).
// If a health check is configured, it waits for the container to become healthy.
func (e *Engine) CreateAndStart(ctx context.Context, opts CreateOpts) (string, error) {
	// Remove existing container with same name for idempotency.
	if err := e.Remove(ctx, opts.Name); err != nil {
		return "", fmt.Errorf("removing existing container: %w", err)
	}

	// Build port bindings.
	exposedPorts := nat.PortSet{}
	portBindings := nat.PortMap{}
	for hostPort, containerPort := range opts.Ports {
		cp := nat.Port(strconv.Itoa(containerPort) + "/tcp")
		exposedPorts[cp] = struct{}{}
		portBindings[cp] = []nat.PortBinding{
			{HostIP: "0.0.0.0", HostPort: strconv.Itoa(hostPort)},
		}
	}

	// Build mounts from volumes.
	var mounts []mount.Mount
	for hostPath, containerPath := range opts.Volumes {
		mounts = append(mounts, mount.Mount{
			Type:   mount.TypeBind,
			Source: hostPath,
			Target: containerPath,
		})
	}

	// Build health check config.
	var healthCheck *container.HealthConfig
	hasHealth := len(opts.HealthCmd) > 0
	if hasHealth {
		interval, err := time.ParseDuration(opts.HealthInterval)
		if err != nil || interval <= 0 {
			interval = 2 * time.Second
		}
		timeout, err := time.ParseDuration(opts.HealthTimeout)
		if err != nil || timeout <= 0 {
			timeout = 5 * time.Second
		}
		healthCheck = &container.HealthConfig{
			Test:     opts.HealthCmd,
			Interval: interval,
			Timeout:  timeout,
			Retries:  opts.HealthRetries,
		}
	}

	// Create container.
	containerCfg := &container.Config{
		Image:        opts.Image,
		Env:          opts.Env,
		Cmd:          opts.Cmd,
		ExposedPorts: exposedPorts,
		Labels:       opts.Labels,
		Healthcheck:  healthCheck,
	}

	hostCfg := &container.HostConfig{
		PortBindings: portBindings,
		Mounts:       mounts,
		RestartPolicy: container.RestartPolicy{
			Name: container.RestartPolicyUnlessStopped,
		},
	}

	resp, err := e.client.ContainerCreate(ctx, containerCfg, hostCfg, &network.NetworkingConfig{}, nil, opts.Name)
	if err != nil {
		return "", fmt.Errorf("create container %s: %w", opts.Name, err)
	}

	// Start the container.
	if err := e.client.ContainerStart(ctx, resp.ID, container.StartOptions{}); err != nil {
		return "", fmt.Errorf("start container %s: %w", opts.Name, err)
	}

	// Wait for healthy if health check is configured.
	if hasHealth {
		retries := opts.HealthRetries
		if retries <= 0 {
			retries = 15
		}
		pollInterval, _ := time.ParseDuration(opts.HealthInterval)
		if pollInterval <= 0 {
			pollInterval = 2 * time.Second
		}
		if err := e.waitHealthy(ctx, resp.ID, retries, pollInterval); err != nil {
			return resp.ID, fmt.Errorf("container %s unhealthy: %w", opts.Name, err)
		}
	}

	return resp.ID, nil
}

// Start starts an existing stopped container by name.
func (e *Engine) Start(ctx context.Context, name string) error {
	if err := e.client.ContainerStart(ctx, name, container.StartOptions{}); err != nil {
		return fmt.Errorf("start container %s: %w", name, err)
	}
	return nil
}

// Stop stops a running container with a 10-second timeout.
func (e *Engine) Stop(ctx context.Context, name string) error {
	timeout := 10
	if err := e.client.ContainerStop(ctx, name, container.StopOptions{Timeout: &timeout}); err != nil {
		return fmt.Errorf("stop container %s: %w", name, err)
	}
	return nil
}

// Remove force-removes a container by name, ignoring not-found errors.
func (e *Engine) Remove(ctx context.Context, name string) error {
	err := e.client.ContainerRemove(ctx, name, container.RemoveOptions{Force: true})
	if err != nil && !client.IsErrNotFound(err) {
		return fmt.Errorf("remove container %s: %w", name, err)
	}
	return nil
}

// Exec creates an exec instance in a container, attaches to it, drains the
// output, and returns an error if the command exits with a non-zero code.
func (e *Engine) Exec(ctx context.Context, containerName string, cmd []string) error {
	execCfg := container.ExecOptions{
		Cmd:          cmd,
		AttachStdout: true,
		AttachStderr: true,
	}

	execID, err := e.client.ContainerExecCreate(ctx, containerName, execCfg)
	if err != nil {
		return fmt.Errorf("exec create in %s: %w", containerName, err)
	}

	resp, err := e.client.ContainerExecAttach(ctx, execID.ID, container.ExecAttachOptions{})
	if err != nil {
		return fmt.Errorf("exec attach in %s: %w", containerName, err)
	}
	defer resp.Close()

	// Drain output.
	if _, err := io.Copy(io.Discard, resp.Reader); err != nil {
		return fmt.Errorf("exec drain in %s: %w", containerName, err)
	}

	// Check exit code.
	inspect, err := e.client.ContainerExecInspect(ctx, execID.ID)
	if err != nil {
		return fmt.Errorf("exec inspect in %s: %w", containerName, err)
	}
	if inspect.ExitCode != 0 {
		return fmt.Errorf("exec in %s exited with code %d", containerName, inspect.ExitCode)
	}

	return nil
}

// Logs streams container logs to the given writer with follow enabled, tailing
// the last 100 lines.
func (e *Engine) Logs(ctx context.Context, name string, w io.Writer) error {
	reader, err := e.client.ContainerLogs(ctx, name, container.LogsOptions{
		ShowStdout: true,
		ShowStderr: true,
		Follow:     true,
		Tail:       "100",
	})
	if err != nil {
		return fmt.Errorf("logs for %s: %w", name, err)
	}
	defer reader.Close()

	if _, err := stdcopy.StdCopy(w, w, reader); err != nil {
		return fmt.Errorf("logs stream for %s: %w", name, err)
	}
	return nil
}

// IsRunning inspects a container and returns whether it is currently running.
// Returns false with no error if the container does not exist.
func (e *Engine) IsRunning(ctx context.Context, name string) (bool, error) {
	info, err := e.client.ContainerInspect(ctx, name)
	if err != nil {
		if client.IsErrNotFound(err) {
			return false, nil
		}
		return false, fmt.Errorf("inspect container %s: %w", name, err)
	}
	return info.State.Running, nil
}

// Exists inspects a container and returns true if it exists regardless of state.
// Returns false with no error if the container does not exist.
func (e *Engine) Exists(ctx context.Context, name string) (bool, error) {
	_, err := e.client.ContainerInspect(ctx, name)
	if err != nil {
		if client.IsErrNotFound(err) {
			return false, nil
		}
		return false, fmt.Errorf("inspect container %s: %w", name, err)
	}
	return true, nil
}

// waitHealthy polls the container's health status until it reports "healthy"
// or the retries are exhausted.
func (e *Engine) waitHealthy(ctx context.Context, containerID string, retries int, interval time.Duration) error {
	for i := 0; i < retries; i++ {
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-time.After(interval):
		}

		info, err := e.client.ContainerInspect(ctx, containerID)
		if err != nil {
			return fmt.Errorf("inspect health: %w", err)
		}

		if info.State.Health != nil && info.State.Health.Status == "healthy" {
			return nil
		}
	}
	return fmt.Errorf("not healthy after %d checks", retries)
}
