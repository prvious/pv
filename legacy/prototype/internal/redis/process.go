package redis

import (
	"context"
	"fmt"
	"net"
	"os"
	"strconv"
	"time"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/supervisor"
)

func BuildSupervisorProcess(version string) (supervisor.Process, error) {
	if err := ValidateVersion(version); err != nil {
		return supervisor.Process{}, err
	}
	binPath := ServerBinary(version)
	if _, err := os.Stat(binPath); err != nil {
		return supervisor.Process{}, fmt.Errorf("redis-%s: not installed (run pv redis:install %s)", version, version)
	}
	return supervisor.Process{
		Name:         "redis-" + version,
		Binary:       binPath,
		Args:         buildRedisArgs(version),
		LogFile:      config.RedisLogPathV(version),
		SysProcAttr:  dropSysProcAttr(),
		Ready:        tcpReady(PortFor(version)),
		ReadyTimeout: 10 * time.Second,
	}, nil
}

func buildRedisArgs(version string) []string {
	return []string{
		"--bind", "127.0.0.1",
		"--port", strconv.Itoa(PortFor(version)),
		"--dir", config.RedisDataDirV(version),
		"--dbfilename", "dump.rdb",
		"--pidfile", "/tmp/pv-redis-" + version + ".pid",
		"--daemonize", "no",
		"--protected-mode", "no",
		"--appendonly", "no",
	}
}

func tcpReady(port int) func(context.Context) error {
	addr := fmt.Sprintf("127.0.0.1:%d", port)
	return func(ctx context.Context) error {
		d := net.Dialer{Timeout: 500 * time.Millisecond}
		c, err := d.DialContext(ctx, "tcp", addr)
		if err != nil {
			return err
		}
		c.Close()
		return nil
	}
}
