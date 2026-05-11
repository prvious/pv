package initgen

import (
	"fmt"
	"strings"
)

func laravel(opts Options) string {
	var b strings.Builder

	fmt.Fprintf(&b, "php: %q\n\n", opts.PHP)

	b.WriteString("# Additional hostnames Caddy will serve for this project, each\n")
	b.WriteString("# with its own TLS cert. Hostnames outside *.{project}.test\n")
	b.WriteString("# (the wildcard SAN) make the most sense here.\n")
	b.WriteString("aliases:\n")
	b.WriteString("  # - admin.")
	b.WriteString(opts.ProjectName)
	b.WriteString(".test\n\n")

	b.WriteString("env:\n")
	b.WriteString("  APP_URL: \"{{ .site_url }}\"\n\n")

	switch {
	case opts.Postgres != "":
		writePostgresBlock(&b, opts)
		b.WriteString("\n")
	case opts.Mysql != "":
		writeMysqlBlock(&b, opts)
		b.WriteString("\n")
	}

	b.WriteString("# Each line runs in its own bash -c with the pinned PHP on PATH.\n")
	b.WriteString("# Fail-fast on first non-zero exit.\n")
	b.WriteString("setup:\n")
	b.WriteString("  - cp .env.example .env\n")
	if opts.Postgres != "" {
		fmt.Fprintf(&b, "  - pv postgres:db:create %s\n", opts.ProjectName)
	}
	if opts.Mysql != "" {
		fmt.Fprintf(&b, "  - pv mysql:db:create %s\n", opts.ProjectName)
	}
	b.WriteString("  - composer install\n")
	b.WriteString("  - php artisan key:generate\n")
	if opts.Postgres != "" || opts.Mysql != "" {
		b.WriteString("  - php artisan migrate\n")
	}

	return b.String()
}

// writePostgresBlock emits the postgresql: block with Laravel-shaped
// env keys. Caller is responsible for the trailing blank line.
func writePostgresBlock(b *strings.Builder, opts Options) {
	fmt.Fprintf(b, "postgresql:\n  version: %q\n  env:\n", opts.Postgres)
	b.WriteString("    DB_CONNECTION: pgsql\n")
	b.WriteString("    DB_HOST: \"{{ .host }}\"\n")
	b.WriteString("    DB_PORT: \"{{ .port }}\"\n")
	fmt.Fprintf(b, "    DB_DATABASE: %s\n", opts.ProjectName)
	b.WriteString("    DB_USERNAME: \"{{ .username }}\"\n")
	b.WriteString("    DB_PASSWORD: \"{{ .password }}\"\n")
}

// writeMysqlBlock emits the mysql: block with Laravel-shaped env keys.
func writeMysqlBlock(b *strings.Builder, opts Options) {
	fmt.Fprintf(b, "mysql:\n  version: %q\n  env:\n", opts.Mysql)
	b.WriteString("    DB_CONNECTION: mysql\n")
	b.WriteString("    DB_HOST: \"{{ .host }}\"\n")
	b.WriteString("    DB_PORT: \"{{ .port }}\"\n")
	fmt.Fprintf(b, "    DB_DATABASE: %s\n", opts.ProjectName)
	b.WriteString("    DB_USERNAME: \"{{ .username }}\"\n")
	b.WriteString("    DB_PASSWORD: \"{{ .password }}\"\n")
}
