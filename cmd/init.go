package cmd

import (
	"encoding/json"
	"fmt"
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
		if _, statErr := os.Stat(ymlPath); statErr == nil && !initForce {
			return fmt.Errorf("pv.yml already exists at %s — pass --force to overwrite", ymlPath)
		}

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
		if err := os.WriteFile(ymlPath, []byte(body), 0o644); err != nil {
			return fmt.Errorf("write pv.yml: %w", err)
		}

		ui.Success(fmt.Sprintf("Generated %s", ymlPath))
		ui.Subtle(fmt.Sprintf("Detected project type: %s", labelForType(projectType)))
		ui.Subtle("Review the file and adjust before running `pv link`.")
		return nil
	},
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
	if err == nil && settings != nil && settings.Defaults.PHP != "" {
		return settings.Defaults.PHP
	}
	return "8.4"
}

// composerVersionRe matches "^8.4", "^8.4.0", "~8.4", "~8.4.0",
// ">=8.4", ">=8.4.0" prefixes and captures the major.minor. Anything
// else (compound constraints with `|`, exact pins like "8.4.10", etc.)
// returns "" — fall back to the global default rather than guess wrong.
var composerVersionRe = regexp.MustCompile(`^(?:\^|~|>=)?\s*(\d+\.\d+)`)

// phpFromComposer reads composer.json's require.php and returns a
// concrete major.minor when the constraint can be cleanly parsed.
// Returns "" on any parse failure.
func phpFromComposer(projectPath string) string {
	raw, err := os.ReadFile(filepath.Join(projectPath, "composer.json"))
	if err != nil {
		return ""
	}
	var parsed struct {
		Require map[string]string `json:"require"`
	}
	if err := json.Unmarshal(raw, &parsed); err != nil {
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

// resolveInitPostgres returns the highest installed postgres major,
// or "" if mysql is preferred (and installed) OR if no postgres is
// installed.
func resolveInitPostgres(preferMysql bool) string {
	majors, _ := postgres.InstalledMajors()
	if len(majors) == 0 {
		return ""
	}
	if preferMysql {
		versions, _ := mysql.InstalledVersions()
		if len(versions) > 0 {
			return ""
		}
	}
	return majors[len(majors)-1]
}

// resolveInitMysql returns the highest installed mysql version, or
// "" when postgres should win (postgres installed AND preferMysql is
// false) or no mysql is installed.
func resolveInitMysql(preferMysql bool) string {
	versions, _ := mysql.InstalledVersions()
	if len(versions) == 0 {
		return ""
	}
	if !preferMysql {
		majors, _ := postgres.InstalledMajors()
		if len(majors) > 0 {
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
