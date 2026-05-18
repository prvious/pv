package cmd

import (
	"encoding/json"
	"errors"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"regexp"
	"strings"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/detection"
	"github.com/prvious/pv/internal/initgen"
	"github.com/prvious/pv/internal/mysql"
	"github.com/prvious/pv/internal/postgres"
	"github.com/prvious/pv/internal/projectenv"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var (
	initForce bool
	initMysql bool
)

var initCmd = &cobra.Command{
	Use:     "init [path]",
	GroupID: "core",
	Short:   "Generate a default pv.yml for the project",
	Long: `Detect the project type and write a pv.yml with sensible defaults.
Refuses to overwrite an existing pv.yml unless --force is set.

Designed to be reviewed and committed: the file is the contract
between your project and pv.`,
	Example: `  pv init
  pv init /path/to/project
  pv init --mysql           # prefer mysql when both postgres + mysql are installed
  pv init --force           # overwrite an existing pv.yml`,
	Args: cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		projectPath, err := resolveInitPath(args)
		if err != nil {
			return err
		}

		ymlPath := filepath.Join(projectPath, config.ProjectConfigFilename)

		projectType := detection.Detect(projectPath)
		projectName := projectenv.SanitizeProjectName(filepath.Base(projectPath))

		opts := initgen.Options{
			ProjectType: projectType,
			ProjectName: projectName,
			PHP:         resolveInitPHP(projectPath),
			Postgres:    resolveInitPostgres(initMysql),
			Mysql:       resolveInitMysql(initMysql),
		}

		body := initgen.Generate(opts)
		if err := writePvYml(ymlPath, []byte(body), initForce); err != nil {
			return err
		}

		ui.Success(fmt.Sprintf("Generated %s", ymlPath))
		ui.Subtle(fmt.Sprintf("Detected project type: %s", labelForType(projectType)))
		ui.Subtle("Review the file and adjust before running `pv link`.")
		return nil
	},
}

// writePvYml atomically writes the generated pv.yml. When force is
// false, the write is create-only (O_EXCL) and fails with a clear
// "already exists" error — no stat-then-write race. When force is
// true, the write goes through a sibling temp file + rename so a
// crash or disk-full mid-write doesn't corrupt the user's existing
// pv.yml.
func writePvYml(path string, body []byte, force bool) error {
	if !force {
		// Create-only. EEXIST is the "user has a pv.yml" signal; any
		// other error (perms, broken parent) propagates verbatim.
		f, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o644)
		if err != nil {
			if errors.Is(err, fs.ErrExist) {
				return fmt.Errorf("pv.yml already exists at %s — pass --force to overwrite", path)
			}
			return fmt.Errorf("create pv.yml: %w", err)
		}
		defer f.Close()
		if _, err := f.Write(body); err != nil {
			return fmt.Errorf("write pv.yml: %w", err)
		}
		return f.Sync()
	}

	// Force path: write to sibling temp, then rename. Rename within the
	// same dir is atomic on every filesystem that ships with macOS or
	// Linux runners.
	dir := filepath.Dir(path)
	tmp, err := os.CreateTemp(dir, ".pv.yml.tmp-*")
	if err != nil {
		return fmt.Errorf("create temp pv.yml: %w", err)
	}
	tmpPath := tmp.Name()
	defer os.Remove(tmpPath) // best-effort cleanup if rename fails

	if _, err := tmp.Write(body); err != nil {
		tmp.Close()
		return fmt.Errorf("write temp pv.yml: %w", err)
	}
	if err := tmp.Sync(); err != nil {
		tmp.Close()
		return fmt.Errorf("sync temp pv.yml: %w", err)
	}
	if err := tmp.Close(); err != nil {
		return fmt.Errorf("close temp pv.yml: %w", err)
	}
	if err := os.Chmod(tmpPath, 0o644); err != nil {
		return fmt.Errorf("chmod temp pv.yml: %w", err)
	}
	if err := os.Rename(tmpPath, path); err != nil {
		return fmt.Errorf("rename pv.yml: %w", err)
	}
	return nil
}

// resolveInitPath returns the absolute path of the project we're
// generating pv.yml for: the args[0] if given, else cwd. Validates
// that the path is a directory.
func resolveInitPath(args []string) (string, error) {
	raw := "."
	if len(args) > 0 {
		raw = args[0]
	}
	abs, err := filepath.Abs(raw)
	if err != nil {
		return "", fmt.Errorf("resolve path %q: %w", raw, err)
	}
	info, err := os.Stat(abs)
	if err != nil {
		return "", fmt.Errorf("stat %s: %w", abs, err)
	}
	if !info.IsDir() {
		return "", fmt.Errorf("%s is not a directory", abs)
	}
	return abs, nil
}

// resolveInitPHP returns the PHP version to pin in the generated
// pv.yml. Prefers composer.json's require.php constraint when it
// pins cleanly to a major.minor; falls back to the user's global
// default in settings; final fallback is "8.4".
func resolveInitPHP(projectPath string) string {
	if v := phpFromComposer(projectPath); v != "" {
		return v
	}
	settings, err := config.LoadSettings()
	if err != nil {
		ui.Subtle(fmt.Sprintf("Could not load settings: %v — using default PHP", err))
	} else if settings != nil && settings.Defaults.PHP != "" {
		return settings.Defaults.PHP
	}
	return "8.4"
}

// composerVersionRe captures the leading "major.minor" of a composer
// require.php constraint, with an optional "^", "~", or ">=" prefix.
// Greedy on the first token only — compound constraints like
// "^8.3 || ^8.4" yield "8.3"; exact pins like "8.4.10" yield "8.4"
// (taking the first major.minor as a usable hint). Returns no match
// for inputs that don't expose a "<major>.<minor>" token after an
// optional supported prefix — including bare majors like "8",
// non-supported operators like ">7.4", and non-numeric constraints
// like "@dev" or "". In those cases the caller falls back to the
// global default.
var composerVersionRe = regexp.MustCompile(`^(?:\^|~|>=)?\s*(\d+\.\d+)`)

// phpFromComposer reads composer.json's require.php and returns the
// leading "major.minor" captured by composerVersionRe. Returns "" on
// any I/O / JSON parse failure or when the constraint doesn't expose
// a numeric token. Best-effort hint, not a strict version pinner —
// the caller should fall back to settings.Defaults.PHP when this
// returns "".
func phpFromComposer(projectPath string) string {
	composerPath := filepath.Join(projectPath, "composer.json")
	raw, err := os.ReadFile(composerPath)
	if err != nil {
		if !errors.Is(err, fs.ErrNotExist) {
			ui.Subtle(fmt.Sprintf("Could not read composer.json: %v — using default PHP", err))
		}
		return ""
	}
	var parsed struct {
		Require map[string]string `json:"require"`
	}
	if err := json.Unmarshal(raw, &parsed); err != nil {
		ui.Subtle(fmt.Sprintf("composer.json is unparseable: %v — using default PHP", err))
		return ""
	}
	constraint := strings.TrimSpace(parsed.Require["php"])
	if constraint == "" {
		return ""
	}
	m := composerVersionRe.FindStringSubmatch(constraint)
	if len(m) < 2 {
		return ""
	}
	return m[1]
}

// installedPostgresMajors wraps postgres.InstalledMajors with a
// ui.Subtle warning on real errors so the caller doesn't have to
// repeat the boilerplate at every call site. Returns the (possibly
// nil) slice in all cases — listing failure is non-fatal because
// init's job is to emit a best-effort default.
func installedPostgresMajors() []string {
	majors, err := postgres.InstalledMajors()
	if err != nil {
		ui.Subtle(fmt.Sprintf("Could not list installed postgres versions: %v", err))
	}
	return majors
}

// installedMysqlVersions mirrors installedPostgresMajors for mysql.
func installedMysqlVersions() []string {
	versions, err := mysql.InstalledVersions()
	if err != nil {
		ui.Subtle(fmt.Sprintf("Could not list installed mysql versions: %v", err))
	}
	return versions
}

// resolveInitPostgres returns the highest installed postgres major,
// or "" if mysql is preferred (and installed) OR if no postgres is
// installed.
func resolveInitPostgres(preferMysql bool) string {
	majors := installedPostgresMajors()
	if len(majors) == 0 {
		return ""
	}
	if preferMysql {
		if len(installedMysqlVersions()) > 0 {
			return ""
		}
	}
	return majors[len(majors)-1]
}

// resolveInitMysql returns the highest installed mysql version, or
// "" when postgres should win (postgres installed AND preferMysql is
// false) or no mysql is installed.
func resolveInitMysql(preferMysql bool) string {
	versions := installedMysqlVersions()
	if len(versions) == 0 {
		return ""
	}
	if !preferMysql {
		if len(installedPostgresMajors()) > 0 {
			return ""
		}
	}
	return versions[len(versions)-1]
}

func labelForType(t string) string {
	switch t {
	case "laravel-octane":
		return "Laravel + Octane"
	case "laravel":
		return "Laravel"
	case "php":
		return "Generic PHP / Composer"
	case "static":
		return "Static site"
	default:
		return "Unknown"
	}
}

func init() {
	initCmd.Flags().BoolVarP(&initForce, "force", "f", false, "Overwrite an existing pv.yml")
	initCmd.Flags().BoolVar(&initMysql, "mysql", false, "Prefer MySQL when both postgres and mysql are installed")
	rootCmd.AddCommand(initCmd)
}
