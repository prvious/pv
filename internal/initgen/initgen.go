// Package initgen builds default pv.yml content for newly-detected
// projects. Per-type templates hand-build YAML so the generated file
// carries comments and stable ordering — yaml.Marshal would lose both.
package initgen

// Options captures everything Generate needs to produce a per-type
// pv.yml. ProjectName must already be sanitized (call
// projectenv.SanitizeProjectName upstream).
type Options struct {
	// ProjectType matches detection.Detect(): "laravel-octane",
	// "laravel", "php", "static", or "" (unknown).
	ProjectType string

	// ProjectName is the sanitized project name. Used as the literal
	// DB_DATABASE value and as the argument to `pv postgres:db:create`
	// in the setup: block.
	ProjectName string

	// PHP is the version string for the top-level `php:` field.
	PHP string

	// Postgres, if non-empty, is the major version (e.g., "18") to
	// generate a postgresql: block for. Empty means "skip postgres
	// block."
	Postgres string

	// Mysql, if non-empty, is the version string (e.g., "8.4") to
	// generate a mysql: block for. Empty means "skip mysql block."
	// Postgres takes precedence if both are set; caller is responsible
	// for picking one.
	Mysql string
}

// Generate returns the YAML string for opts. Always emits valid YAML
// the existing LoadProjectConfig can parse round-trip.
func Generate(opts Options) string {
	switch opts.ProjectType {
	case "laravel", "laravel-octane":
		return laravel(opts)
	case "php":
		return php(opts)
	case "static":
		return static(opts)
	default:
		return unknown(opts)
	}
}
