package config

import (
	"fmt"
	"os"
	"path/filepath"
)

func PvDir() string {
	home, _ := os.UserHomeDir()
	return filepath.Join(home, ".pv")
}

func ConfigDir() string {
	return filepath.Join(PvDir(), "config")
}

func SitesDir() string {
	return filepath.Join(ConfigDir(), "sites")
}

func LogsDir() string {
	return filepath.Join(PvDir(), "logs")
}

func DataDir() string {
	return filepath.Join(PvDir(), "data")
}

func BinDir() string {
	return filepath.Join(PvDir(), "bin")
}

func RegistryPath() string {
	return filepath.Join(DataDir(), "registry.json")
}

func PidFilePath() string {
	return filepath.Join(DataDir(), "pv.pid")
}

func CaddyLogPath() string {
	return filepath.Join(LogsDir(), "caddy.log")
}

func DaemonLogPath() string {
	return filepath.Join(LogsDir(), "pv.log")
}

func DaemonErrLogPath() string {
	return filepath.Join(LogsDir(), "pv.err.log")
}

func CaddyLogPathForVersion(version string) string {
	return filepath.Join(LogsDir(), "caddy-"+version+".log")
}

func CaddyStderrPath() string {
	return filepath.Join(LogsDir(), "caddy-stderr.log")
}

func CaddyStderrPathForVersion(version string) string {
	return filepath.Join(LogsDir(), "caddy-"+version+"-stderr.log")
}

const DNSPort = 10053

func ComposerDir() string {
	return filepath.Join(PvDir(), "composer")
}

func ComposerCacheDir() string {
	return filepath.Join(ComposerDir(), "cache")
}

func ComposerBinDir() string {
	return filepath.Join(ComposerDir(), "vendor", "bin")
}

func ComposerPharPath() string {
	return filepath.Join(InternalBinDir(), "composer.phar")
}

func MagoPath() string {
	return filepath.Join(InternalBinDir(), "mago")
}

func PhpDir() string {
	return filepath.Join(PvDir(), "php")
}

func PhpVersionDir(version string) string {
	return filepath.Join(PhpDir(), version)
}

func PhpEtcDir(version string) string {
	return filepath.Join(PhpVersionDir(version), "etc")
}

func PhpConfDDir(version string) string {
	return filepath.Join(PhpVersionDir(version), "conf.d")
}

func PhpSessionDir(version string) string {
	return filepath.Join(DataDir(), "sessions", version)
}

func PhpTmpDir(version string) string {
	return filepath.Join(DataDir(), "tmp", version)
}

// PhpEnv returns env vars that point a PHP/FrankenPHP process at the
// per-version php.ini and conf.d.
func PhpEnv(version string) []string {
	return []string{
		"PHPRC=" + PhpEtcDir(version),
		"PHP_INI_SCAN_DIR=" + PhpConfDDir(version),
	}
}

func VersionSitesDir(version string) string {
	return filepath.Join(ConfigDir(), "sites-"+version)
}

func VersionCaddyfilePath(version string) string {
	return filepath.Join(ConfigDir(), "php-"+version+".Caddyfile")
}

// PortForVersion returns the HTTP port for a secondary FrankenPHP instance.
// Scheme: 8000 + major*100 + minor*10, e.g. PHP 8.3 → 8830, PHP 8.4 → 8840.
func PortForVersion(version string) int {
	var major, minor int
	fmt.Sscanf(version, "%d.%d", &major, &minor)
	return 8000 + major*100 + minor*10
}

func VersionsPath() string {
	return filepath.Join(DataDir(), "versions.json")
}

func StatePath() string {
	return filepath.Join(DataDir(), "state.json")
}

func SettingsPath() string {
	return filepath.Join(PvDir(), "pv.yml")
}

func CaddyfilePath() string {
	return filepath.Join(ConfigDir(), "Caddyfile")
}

// CaddyEnv returns environment variable strings that direct Caddy to store
// data under ~/.pv/caddy/ instead of platform-default directories, and
// isolate Composer under ~/.pv/composer/.
func CaddyEnv() []string {
	pvDir := PvDir()
	return []string{
		"XDG_DATA_HOME=" + pvDir,
		"XDG_CONFIG_HOME=" + pvDir,
		"COMPOSER_HOME=" + ComposerDir(),
		"COMPOSER_CACHE_DIR=" + ComposerCacheDir(),
	}
}

// CACertPath returns the path to Caddy's local CA root certificate.
func CACertPath() string {
	return filepath.Join(PvDir(), "caddy", "pki", "authorities", "local", "root.crt")
}

// CAKeyPath returns the path to Caddy's local CA private key.
func CAKeyPath() string {
	return filepath.Join(PvDir(), "caddy", "pki", "authorities", "local", "root.key")
}

func ServicesDir() string {
	return filepath.Join(PvDir(), "services")
}

func ServiceDataDir(service, version string) string {
	return filepath.Join(ServicesDir(), service, version, "data")
}

func InternalBinDir() string {
	return filepath.Join(PvDir(), "internal", "bin")
}

func PackagesDir() string {
	return filepath.Join(PvDir(), "internal", "packages")
}

func ColimaPath() string {
	return filepath.Join(InternalBinDir(), "colima")
}

// ColimaHomeDir returns the directory used as COLIMA_HOME, keeping all Colima
// and Lima state under ~/.pv/ instead of the default ~/.colima/.
func ColimaHomeDir() string {
	return filepath.Join(PvDir(), "internal", "colima")
}

func LimaDir() string {
	return filepath.Join(PvDir(), "internal", "lima")
}

func LimaBinDir() string {
	return filepath.Join(LimaDir(), "bin")
}

func ColimaSocketPath() string {
	return filepath.Join(ColimaHomeDir(), "pv", "docker.sock")
}

func PostgresDir() string {
	return filepath.Join(PvDir(), "postgres")
}

func PostgresVersionDir(major string) string {
	return filepath.Join(PostgresDir(), major)
}

func PostgresBinDir(major string) string {
	return filepath.Join(PostgresVersionDir(major), "bin")
}

func PostgresLogPath(major string) string {
	return filepath.Join(LogsDir(), "postgres-"+major+".log")
}

// MysqlDir is the root for native mysql binary trees:
// ~/.pv/mysql/<version>/{bin,lib,share}.
func MysqlDir() string {
	return filepath.Join(PvDir(), "mysql")
}

// MysqlVersionDir is the per-version root inside MysqlDir.
func MysqlVersionDir(version string) string {
	return filepath.Join(MysqlDir(), version)
}

// MysqlBinDir holds mysqld + mysql + mysqldump etc. for a version.
func MysqlBinDir(version string) string {
	return filepath.Join(MysqlVersionDir(version), "bin")
}

// MysqlDataDir is the per-version mysqld data dir, kept under
// ~/.pv/data/mysql/<version>/ so it survives a binary uninstall (unless
// --force is used).
func MysqlDataDir(version string) string {
	return filepath.Join(DataDir(), "mysql", version)
}

// MysqlLogPath returns the supervisor log file for a mysql version.
func MysqlLogPath(version string) string {
	return filepath.Join(LogsDir(), "mysql-"+version+".log")
}

// RedisDir is the root for the native redis binary tree:
// ~/.pv/redis/{redis-server,redis-cli}.
// Single-version — no per-version subdir.
func RedisDir() string {
	return filepath.Join(PvDir(), "redis")
}

// RedisDataDir is the redis-server data dir, kept under
// ~/.pv/data/redis/ so it survives a binary uninstall (unless
// --force is used). RDB snapshots land in <RedisDataDir>/dump.rdb.
func RedisDataDir() string {
	return filepath.Join(DataDir(), "redis")
}

// RedisLogPath returns the supervisor log file for redis.
func RedisLogPath() string {
	return filepath.Join(LogsDir(), "redis.log")
}

func EnsureDirs() error {
	dirs := []string{
		ConfigDir(),
		SitesDir(),
		LogsDir(),
		DataDir(),
		BinDir(),
		PhpDir(),
		ComposerDir(),
		ComposerCacheDir(),
		ServicesDir(),
		InternalBinDir(),
		PackagesDir(),
		ColimaHomeDir(),
		MysqlDir(),
		RedisDir(),
		RedisDataDir(),
	}
	for _, dir := range dirs {
		if err := os.MkdirAll(dir, 0755); err != nil {
			return err
		}
	}
	return nil
}
