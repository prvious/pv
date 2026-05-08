package mysql

import (
	"context"
	"fmt"
	"net"
	"os"
	"path/filepath"
	"strconv"
	"time"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/supervisor"
)

// BuildSupervisorProcess returns a supervisor.Process for a mysql version.
// Refuses to build for a data dir without auto.cnf (i.e., not yet
// --initialize-insecure'd). All boot configuration is on the command line —
// no my.cnf — so pv is the single source of truth.
func BuildSupervisorProcess(version string) (supervisor.Process, error) {
	dataDir := config.MysqlDataDir(version)
	if _, err := os.Stat(filepath.Join(dataDir, "auto.cnf")); err != nil {
		return supervisor.Process{}, fmt.Errorf("mysql %s: data dir not initialized (run pv mysql:install %s)", version, version)
	}
	port, err := PortFor(version)
	if err != nil {
		return supervisor.Process{}, err
	}
	binary := filepath.Join(config.MysqlBinDir(version), "mysqld")
	args := buildMysqldArgs(version, dataDir, port)
	return supervisor.Process{
		Name:         "mysql-" + version,
		Binary:       binary,
		Args:         args,
		LogFile:      config.MysqlLogPath(version),
		SysProcAttr:  dropSysProcAttr(),
		Ready:        tcpReady(port),
		ReadyTimeout: 30 * time.Second,
	}, nil
}

// buildMysqldArgs returns the flag set passed to mysqld at boot. Single
// source of truth: no my.cnf, no my.cnf.d, no /etc/my.cnf — every knob
// pv cares about is here. --mysqlx=OFF disables the X Protocol port
// (default 33060) so two majors don't collide on it; --skip-name-resolve
// avoids reverse-DNS waits on a loopback connection.
func buildMysqldArgs(version, dataDir string, port int) []string {
	basedir := config.MysqlVersionDir(version)
	return []string{
		"--datadir=" + dataDir,
		"--basedir=" + basedir,
		"--port=" + strconv.Itoa(port),
		"--bind-address=127.0.0.1",
		"--socket=/tmp/pv-mysql-" + version + ".sock",
		"--pid-file=/tmp/pv-mysql-" + version + ".pid",
		"--log-error=" + config.MysqlLogPath(version),
		"--mysqlx=OFF",
		"--skip-name-resolve",
	}
}

// tcpReady returns a Ready function that probes 127.0.0.1:port. mysqld
// binds the listener late in boot (after InnoDB recovery), so this is
// the right signal — earlier checks like "pid file present" can fire
// before the server accepts connections.
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
