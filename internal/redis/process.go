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

// BuildSupervisorProcess returns a supervisor.Process for redis. Refuses
// to build when the binary is missing — the supervisor would just fail
// to exec and we want a clearer error.
//
// All boot configuration is on the command line — no redis.conf — so pv
// is the single source of truth.
func BuildSupervisorProcess() (supervisor.Process, error) {
	binPath := ServerBinary()
	if _, err := os.Stat(binPath); err != nil {
		return supervisor.Process{}, fmt.Errorf("redis: not installed (run pv redis:install)")
	}
	return supervisor.Process{
		Name:         "redis",
		Binary:       binPath,
		Args:         buildRedisArgs(),
		LogFile:      config.RedisLogPath(),
		SysProcAttr:  dropSysProcAttr(),
		Ready:        tcpReady(PortFor()),
		ReadyTimeout: 10 * time.Second,
	}, nil
}

// buildRedisArgs returns the flag set passed to redis-server at boot.
// Single source of truth: no redis.conf — every knob pv cares about is
// here.
//
// We deliberately do NOT pass --logfile: the supervisor opens
// RedisLogPath as the parent (running as root) and inherits the fd to
// the child, which sidesteps the ownership problem of the dropped
// redis-server process trying to open a root-owned log file itself.
// redis-server's stderr is captured via that inherited fd. Same fix we
// applied to mysql.
//
// Compiled-in save policy stays in effect (3600 1 / 300 100 / 60 10000).
// AOF off — RDB is sufficient for dev work.
func buildRedisArgs() []string {
	return []string{
		"--bind", "127.0.0.1",
		"--port", strconv.Itoa(PortFor()),
		"--dir", config.RedisDataDir(),
		"--dbfilename", "dump.rdb",
		"--pidfile", "/tmp/pv-redis.pid",
		"--daemonize", "no",
		"--protected-mode", "no",
		"--appendonly", "no",
	}
}

// tcpReady returns a Ready function that probes 127.0.0.1:port.
// redis-server starts accepting connections almost immediately after
// the listener binds — 10s is generous.
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
