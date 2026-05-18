package cmd

import (
	"fmt"
	"net"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/miekg/dns"
	"github.com/prvious/pv/internal/caddy"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/daemon"
	"github.com/prvious/pv/internal/phpenv"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/setup"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

type check struct {
	Name    string
	Status  bool
	Message string // shown on failure
	Fix     string // suggested fix command
}

var doctorCmd = &cobra.Command{
	Use:     "doctor",
	GroupID: "core",
	Short:   "Diagnose pv installation health",
	Example: "  pv doctor",
	RunE: func(cmd *cobra.Command, args []string) error {
		settings, err := config.LoadSettings()
		if err != nil {
			return fmt.Errorf("cannot load settings: %w", err)
		}

		reg, err := registry.Load()
		if err != nil {
			return fmt.Errorf("cannot load registry: %w", err)
		}

		versions, _ := phpenv.InstalledVersions()
		globalPHP := settings.Defaults.PHP

		var allChecks []sectionResult

		allChecks = append(allChecks, runBinaryChecks(globalPHP, versions))
		allChecks = append(allChecks, runEnvironmentChecks())
		allChecks = append(allChecks, runComposerIsolationChecks())
		allChecks = append(allChecks, runNetworkChecks(settings))
		allChecks = append(allChecks, runServerChecks(globalPHP, reg))
		allChecks = append(allChecks, runProjectChecks(settings, reg, globalPHP))

		ui.SectionHeader("pv doctor")

		passed, failed := 0, 0
		for _, section := range allChecks {
			ui.SectionHeader(section.Name)
			for _, c := range section.Checks {
				if c.Status {
					ui.Success(c.Name)
					passed++
				} else {
					ui.Fail(c.Name)
					if c.Message != "" {
						ui.FailDetail(c.Message)
					}
					if c.Fix != "" {
						ui.FailDetail("→ Run: " + c.Fix)
					}
					failed++
				}
			}
		}

		if failed > 0 {
			ui.Fail(fmt.Sprintf("%d passed, %d issues found", passed, failed))
			return ui.ErrAlreadyPrinted
		}
		ui.Success(fmt.Sprintf("%d passed, no issues found", passed))
		return nil
	},
}

type sectionResult struct {
	Name   string
	Checks []check
}

func init() {
	rootCmd.AddCommand(doctorCmd)
}

// --- Binary Checks ---

func runBinaryChecks(globalPHP string, versions []string) sectionResult {
	var checks []check

	// Check each installed PHP version has both binaries.
	for _, v := range versions {
		fpPath := phpenv.FrankenPHPPath(v)
		phpPath := phpenv.PHPPath(v)

		fpOk := isExecutable(fpPath)
		phpOk := isExecutable(phpPath)

		if fpOk && phpOk {
			label := fmt.Sprintf("PHP %s (frankenphp + php)", v)
			if v == globalPHP {
				label += " [global]"
			}
			checks = append(checks, check{Name: label, Status: true})
		} else {
			var missing []string
			if !fpOk {
				missing = append(missing, "frankenphp")
			}
			if !phpOk {
				missing = append(missing, "php")
			}
			checks = append(checks, check{
				Name:    fmt.Sprintf("PHP %s", v),
				Status:  false,
				Message: fmt.Sprintf("missing: %s", strings.Join(missing, ", ")),
				Fix:     fmt.Sprintf("pv php:install %s", v),
			})
		}
	}

	if len(versions) == 0 {
		checks = append(checks, check{
			Name:    "PHP versions",
			Status:  false,
			Message: "no PHP versions installed",
			Fix:     "pv php:install 8.4",
		})
	}

	// Composer.
	composerPath := config.ComposerPharPath()
	if isExecutable(composerPath) || fileExists(composerPath) {
		checks = append(checks, check{Name: "Composer", Status: true})
	} else {
		checks = append(checks, check{
			Name:    "Composer",
			Status:  false,
			Message: "composer.phar not found",
			Fix:     "pv install",
		})
	}

	// Mago.
	magoPath := filepath.Join(config.BinDir(), "mago")
	if isExecutable(magoPath) {
		checks = append(checks, check{Name: "Mago", Status: true})
	} else {
		checks = append(checks, check{
			Name:    "Mago",
			Status:  false,
			Message: "mago not found",
			Fix:     "pv mago:install",
		})
	}

	return sectionResult{Name: "Binaries", Checks: checks}
}

// --- Environment Checks ---

func runEnvironmentChecks() sectionResult {
	var checks []check

	binDir := config.BinDir()
	composerBinDir := config.ComposerBinDir()
	pathEnv := os.Getenv("PATH")
	pathDirs := filepath.SplitList(pathEnv)

	// ~/.pv/bin in PATH.
	if containsPath(pathDirs, binDir) {
		checks = append(checks, check{Name: "~/.pv/bin on PATH", Status: true})
	} else {
		checks = append(checks, check{
			Name:    "~/.pv/bin not on PATH",
			Status:  false,
			Message: "pv binaries won't be found",
			Fix:     fmt.Sprintf("Add to your shell config: export PATH=\"%s:$PATH\"", binDir),
		})
	}

	// ~/.pv/composer/vendor/bin in PATH.
	if containsPath(pathDirs, composerBinDir) {
		checks = append(checks, check{Name: "~/.pv/composer/vendor/bin on PATH", Status: true})
	} else {
		checks = append(checks, check{
			Name:    "~/.pv/composer/vendor/bin not on PATH",
			Status:  false,
			Message: "global Composer binaries won't be found",
			Fix:     fmt.Sprintf("Add to your shell config: export PATH=\"%s:$PATH\"", composerBinDir),
		})
	}

	// PHP shim exists.
	phpShim := filepath.Join(binDir, "php")
	if isExecutable(phpShim) {
		checks = append(checks, check{Name: "PHP shim", Status: true})
	} else {
		checks = append(checks, check{
			Name:    "PHP shim",
			Status:  false,
			Message: "~/.pv/bin/php not found or not executable",
			Fix:     "pv install",
		})
	}

	// Composer symlink exists.
	composerLink := filepath.Join(binDir, "composer")
	if _, err := os.Readlink(composerLink); err == nil {
		checks = append(checks, check{Name: "Composer symlink", Status: true})
	} else if isExecutable(composerLink) {
		checks = append(checks, check{Name: "Composer binary", Status: true})
	} else {
		checks = append(checks, check{
			Name:    "Composer symlink",
			Status:  false,
			Message: "~/.pv/bin/composer not found",
			Fix:     "pv install",
		})
	}

	// FrankenPHP symlink.
	fpLink := filepath.Join(binDir, "frankenphp")
	if target, err := os.Readlink(fpLink); err == nil {
		if fileExists(target) {
			checks = append(checks, check{Name: "FrankenPHP symlink", Status: true})
		} else {
			checks = append(checks, check{
				Name:    "FrankenPHP symlink",
				Status:  false,
				Message: fmt.Sprintf("broken symlink → %s", target),
				Fix:     "pv php:use [version]",
			})
		}
	} else if isExecutable(fpLink) {
		checks = append(checks, check{Name: "FrankenPHP binary", Status: true})
	} else {
		checks = append(checks, check{
			Name:    "FrankenPHP symlink",
			Status:  false,
			Message: "~/.pv/bin/frankenphp not found",
			Fix:     "pv install",
		})
	}

	return sectionResult{Name: "Environment", Checks: checks}
}

// --- Composer Isolation Checks ---

func runComposerIsolationChecks() sectionResult {
	var checks []check

	// Check ~/.pv/composer/ directory exists.
	composerDir := config.ComposerDir()
	if dirExists(composerDir) {
		checks = append(checks, check{Name: "Composer home directory", Status: true})
	} else {
		checks = append(checks, check{
			Name:    "Composer home directory",
			Status:  false,
			Message: fmt.Sprintf("%s does not exist", composerDir),
			Fix:     "pv install",
		})
	}

	// Verify COMPOSER_HOME and COMPOSER_CACHE_DIR env vars are set correctly.
	// Isolation is handled by `pv env` which exports these variables.
	composerHomeEnv := os.Getenv("COMPOSER_HOME")
	expectedHome := config.ComposerDir()
	if composerHomeEnv == expectedHome {
		checks = append(checks, check{Name: "COMPOSER_HOME isolated", Status: true})
	} else if composerHomeEnv != "" {
		checks = append(checks, check{
			Name:    "COMPOSER_HOME isolated",
			Status:  false,
			Message: fmt.Sprintf("COMPOSER_HOME is %q, expected %q", composerHomeEnv, expectedHome),
			Fix:     `Add to your shell config: eval "$(pv env)"`,
		})
	} else {
		checks = append(checks, check{
			Name:    "COMPOSER_HOME isolated",
			Status:  false,
			Message: "COMPOSER_HOME not set",
			Fix:     `Add to your shell config: eval "$(pv env)"`,
		})
	}

	composerCacheEnv := os.Getenv("COMPOSER_CACHE_DIR")
	expectedCache := config.ComposerCacheDir()
	if composerCacheEnv == expectedCache {
		checks = append(checks, check{Name: "Composer cache isolated", Status: true})
	} else if composerCacheEnv != "" {
		checks = append(checks, check{
			Name:    "Composer cache isolated",
			Status:  false,
			Message: fmt.Sprintf("COMPOSER_CACHE_DIR is %q, expected %q", composerCacheEnv, expectedCache),
			Fix:     `Add to your shell config: eval "$(pv env)"`,
		})
	} else {
		checks = append(checks, check{
			Name:    "Composer cache isolated",
			Status:  false,
			Message: "COMPOSER_CACHE_DIR not set",
			Fix:     `Add to your shell config: eval "$(pv env)"`,
		})
	}

	// Warn if ~/.composer/ also exists (potential confusion).
	home, _ := os.UserHomeDir()
	systemComposerDir := filepath.Join(home, ".composer")
	if dirExists(systemComposerDir) {
		checks = append(checks, check{
			Name:    "No conflicting ~/.composer",
			Status:  false,
			Message: fmt.Sprintf("%s exists and may cause confusion with pv's isolated Composer", systemComposerDir),
		})
	} else {
		checks = append(checks, check{Name: "No conflicting ~/.composer", Status: true})
	}

	return sectionResult{Name: "Composer", Checks: checks}
}

// --- Network Checks ---

func runNetworkChecks(settings *config.Settings) sectionResult {
	var checks []check

	// DNS resolver file.
	if err := setup.CheckResolverFile(settings.Defaults.TLD); err == nil {
		checks = append(checks, check{Name: "DNS resolver configured", Status: true})
	} else {
		checks = append(checks, check{
			Name:    "DNS resolver",
			Status:  false,
			Message: err.Error(),
			Fix:     "sudo pv install",
		})
	}

	// DNS responding (only if server appears to be running).
	if server.IsRunning() || daemon.IsLoaded() {
		if checkDNSResponding(settings.Defaults.TLD) {
			checks = append(checks, check{Name: "DNS responding", Status: true})
		} else {
			checks = append(checks, check{
				Name:    "DNS responding",
				Status:  false,
				Message: fmt.Sprintf("DNS server not responding on port %d", config.DNSPort),
				Fix:     "pv restart",
			})
		}
	}

	// CA certificate.
	caCertPath := config.CACertPath()
	if fileExists(caCertPath) {
		checks = append(checks, check{Name: "CA certificate exists", Status: true})
	} else {
		checks = append(checks, check{
			Name:    "CA certificate",
			Status:  false,
			Message: "Caddy local CA root certificate not found",
			Fix:     "pv start (will auto-generate on first run)",
		})
	}

	// CA trusted in keychain (macOS).
	if fileExists(caCertPath) {
		if checkCATrusted() {
			checks = append(checks, check{Name: "CA certificate trusted", Status: true})
		} else {
			checks = append(checks, check{
				Name:    "CA certificate trusted",
				Status:  false,
				Message: "Caddy Local Authority not found in system keychain",
				Fix:     "sudo pv install",
			})
		}
	}

	return sectionResult{Name: "Network", Checks: checks}
}

// --- Server Checks ---

func runServerChecks(globalPHP string, reg *registry.Registry) sectionResult {
	var checks []check

	daemonLoaded := daemon.IsLoaded()
	foregroundRunning := server.IsRunning()

	if daemonLoaded {
		pid, err := daemon.GetPID()
		if err == nil && pid > 0 {
			checks = append(checks, check{
				Name:   fmt.Sprintf("Running (PID %d, daemon mode)", pid),
				Status: true,
			})
		} else {
			checks = append(checks, check{
				Name:    "Server",
				Status:  false,
				Message: "launchd service loaded but not running (crashed?)",
				Fix:     "pv restart",
			})
		}
	} else if foregroundRunning {
		pid, _ := server.ReadPID()
		checks = append(checks, check{
			Name:   fmt.Sprintf("Running (PID %d, foreground mode)", pid),
			Status: true,
		})
	} else {
		checks = append(checks, check{
			Name:    "Server not running",
			Status:  false,
			Message: "pv server is not running",
			Fix:     "pv start",
		})
	}

	// If running, check secondary versions that should be active.
	if daemonLoaded || foregroundRunning {
		projects := reg.List()
		activeVersions := caddy.ActiveVersions(projects, globalPHP)
		for version := range activeVersions {
			port := config.PortForVersion(version)
			if checkPortListening(port) {
				checks = append(checks, check{
					Name:   fmt.Sprintf("PHP %s secondary on :%d", version, port),
					Status: true,
				})
			} else {
				checks = append(checks, check{
					Name:    fmt.Sprintf("PHP %s secondary on :%d", version, port),
					Status:  false,
					Message: "port not responding",
					Fix:     "pv restart",
				})
			}
		}
	}

	return sectionResult{Name: "Server", Checks: checks}
}

// --- Project Checks ---

func runProjectChecks(settings *config.Settings, reg *registry.Registry, globalPHP string) sectionResult {
	var checks []check

	// Global PHP version is installed.
	if globalPHP != "" {
		if phpenv.IsInstalled(globalPHP) {
			checks = append(checks, check{
				Name:   fmt.Sprintf("Global PHP %s installed", globalPHP),
				Status: true,
			})
		} else {
			checks = append(checks, check{
				Name:    fmt.Sprintf("Global PHP %s", globalPHP),
				Status:  false,
				Message: "configured global PHP version is not installed",
				Fix:     fmt.Sprintf("pv php:install %s", globalPHP),
			})
		}
	} else {
		checks = append(checks, check{
			Name:    "Global PHP version",
			Status:  false,
			Message: "no global PHP version configured",
			Fix:     "pv php:install 8.4",
		})
	}

	projects := reg.List()
	for _, p := range projects {
		phpV := p.PHP
		if phpV == "" {
			phpV = globalPHP
		}
		if phpV == "" {
			phpV = "none"
		}

		domain := p.Name + "." + settings.Defaults.TLD

		// Check project path exists.
		if !dirExists(p.Path) {
			checks = append(checks, check{
				Name:    fmt.Sprintf("%s → %s (PHP %s)", domain, p.Path, phpV),
				Status:  false,
				Message: "directory missing",
				Fix:     fmt.Sprintf("pv unlink %s", p.Name),
			})
			continue
		}

		// Check resolved PHP version is installed.
		if phpV != "none" && !phpenv.IsInstalled(phpV) {
			checks = append(checks, check{
				Name:    fmt.Sprintf("%s → %s (PHP %s)", domain, p.Path, phpV),
				Status:  false,
				Message: fmt.Sprintf("PHP %s is not installed", phpV),
				Fix:     fmt.Sprintf("pv php:install %s", phpV),
			})
			continue
		}

		// Check site config exists.
		siteConfig := filepath.Join(config.SitesDir(), p.Name+".caddy")
		if !fileExists(siteConfig) {
			checks = append(checks, check{
				Name:    fmt.Sprintf("%s → %s (PHP %s)", domain, p.Path, phpV),
				Status:  false,
				Message: "Caddyfile config missing",
				Fix:     "pv restart",
			})
			continue
		}

		checks = append(checks, check{
			Name:   fmt.Sprintf("%s → %s (PHP %s)", domain, p.Path, phpV),
			Status: true,
		})
	}

	if len(projects) == 0 {
		checks = append(checks, check{
			Name:   "No projects linked",
			Status: true,
		})
	}

	return sectionResult{Name: "Projects", Checks: checks}
}

// --- Service Checks ---

// --- Helpers ---

func fileExists(path string) bool {
	_, err := os.Stat(path)
	return err == nil
}

func dirExists(path string) bool {
	info, err := os.Stat(path)
	return err == nil && info.IsDir()
}

func isExecutable(path string) bool {
	info, err := os.Stat(path)
	if err != nil {
		return false
	}
	return info.Mode()&0111 != 0
}

func containsPath(paths []string, target string) bool {
	for _, p := range paths {
		if p == target {
			return true
		}
	}
	return false
}

func checkDNSResponding(tld string) bool {
	c := new(dns.Client)
	c.Timeout = 2 * time.Second
	m := new(dns.Msg)
	m.SetQuestion("healthcheck."+tld+".", dns.TypeA)

	r, _, err := c.Exchange(m, fmt.Sprintf("127.0.0.1:%d", config.DNSPort))
	if err != nil || r == nil {
		return false
	}

	for _, ans := range r.Answer {
		if a, ok := ans.(*dns.A); ok && a.A.Equal(net.ParseIP("127.0.0.1")) {
			return true
		}
	}
	return false
}

func checkPortListening(port int) bool {
	conn, err := net.Dial("tcp", fmt.Sprintf("127.0.0.1:%d", port))
	if err != nil {
		return false
	}
	conn.Close()
	return true
}

func checkCATrusted() bool {
	trusted, err := setup.IsCATrusted()
	if err != nil {
		return false
	}
	return trusted
}
