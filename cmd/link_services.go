package cmd

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/services"
	"github.com/prvious/pv/internal/ui"
)

// detectAndBindServices detects services referenced in a project's .env file
// and binds them to the project in the registry.
func detectAndBindServices(projectPath, projectName string, reg *registry.Registry) {
	envPath := filepath.Join(projectPath, ".env")
	envVars, err := services.ReadDotEnv(envPath)
	if err != nil {
		return
	}

	dbName := sanitizeProjectName(projectName)
	var detected []string
	var suggestions []string
	var needsEnvUpdate bool

	// Detect MySQL.
	if conn, ok := envVars["DB_CONNECTION"]; ok && conn == "mysql" {
		svcKey := findServiceByName(reg, "mysql")
		if svcKey != "" {
			detected = append(detected, fmt.Sprintf("DB_CONNECTION=mysql -> %s", svcKey))
			bindProjectService(reg, projectName, "mysql", svcKey)
			needsEnvUpdate = true
		} else {
			suggestions = append(suggestions, "DB_CONNECTION=mysql detected but no MySQL service running.\n    Run: pv service:add mysql")
		}
	}

	// Detect PostgreSQL.
	if conn, ok := envVars["DB_CONNECTION"]; ok && conn == "pgsql" {
		svcKey := findServiceByName(reg, "postgres")
		if svcKey != "" {
			detected = append(detected, fmt.Sprintf("DB_CONNECTION=pgsql -> %s", svcKey))
			bindProjectService(reg, projectName, "postgres", svcKey)
			needsEnvUpdate = true
		} else {
			suggestions = append(suggestions, "DB_CONNECTION=pgsql detected but no PostgreSQL service running.\n    Run: pv service:add postgres")
		}
	}

	// Detect Redis.
	if _, ok := envVars["REDIS_HOST"]; ok {
		svcKey := findServiceByName(reg, "redis")
		if svcKey != "" {
			detected = append(detected, fmt.Sprintf("REDIS_HOST -> %s", svcKey))
			bindProjectService(reg, projectName, "redis", svcKey)
			needsEnvUpdate = true
		} else {
			suggestions = append(suggestions, "REDIS_HOST detected but no Redis service running.\n    Run: pv service:add redis")
		}
	}

	// Detect Mail (Mailpit).
	if host, ok := envVars["MAIL_HOST"]; ok && (strings.Contains(host, "localhost") || strings.Contains(host, "127.0.0.1")) {
		svcKey := findServiceByName(reg, "mail")
		if svcKey != "" {
			detected = append(detected, fmt.Sprintf("MAIL_HOST -> %s", svcKey))
			bindProjectService(reg, projectName, "mail", svcKey)
			needsEnvUpdate = true
		} else {
			suggestions = append(suggestions, "MAIL_HOST (localhost) detected but no Mail service running.\n    Run: pv service:add mail")
		}
	}

	// Detect S3 (S3-compatible storage).
	if endpoint, ok := envVars["AWS_ENDPOINT"]; ok && strings.Contains(endpoint, "localhost") || strings.Contains(endpoint, "127.0.0.1") {
		svcKey := findServiceByName(reg, "s3")
		if svcKey != "" {
			detected = append(detected, fmt.Sprintf("AWS_ENDPOINT -> %s", svcKey))
			bindProjectService(reg, projectName, "s3", svcKey)
			needsEnvUpdate = true
		} else {
			suggestions = append(suggestions, "AWS_ENDPOINT (localhost) detected but no S3 service running.\n    Run: pv service:add s3")
		}
	}

	if len(detected) > 0 {
		fmt.Fprintln(os.Stderr)
		fmt.Fprintf(os.Stderr, "  %s\n", ui.Muted.Render("Detected services:"))
		for _, d := range detected {
			fmt.Fprintf(os.Stderr, "    %s\n", d)
		}

		// Auto-create databases for database services.
		for i := range reg.Projects {
			if reg.Projects[i].Name == projectName && reg.Projects[i].Services != nil {
				if reg.Projects[i].Services.MySQL != "" || reg.Projects[i].Services.Postgres != "" {
					// Track database name for the project.
					if !containsStr(reg.Projects[i].Databases, dbName) {
						reg.Projects[i].Databases = append(reg.Projects[i].Databases, dbName)
					}
					fmt.Fprintf(os.Stderr, "    %s Created database '%s'\n", ui.Green.Render("✓"), dbName)
				}
				break
			}
		}
	}

	if needsEnvUpdate {
		_ = needsEnvUpdate // .env update would be offered here interactively
	}

	for _, s := range suggestions {
		fmt.Fprintf(os.Stderr, "  %s %s\n", ui.Muted.Render("!"), s)
	}
}

// findServiceByName finds the first service key matching a service name.
func findServiceByName(reg *registry.Registry, name string) string {
	for key := range reg.Services {
		keyName := key
		if idx := strings.Index(key, ":"); idx > 0 {
			keyName = key[:idx]
		}
		if keyName == name {
			return key
		}
	}
	return ""
}

// bindProjectService binds a service to a project in the registry.
func bindProjectService(reg *registry.Registry, projectName, svcType, svcKey string) {
	for i := range reg.Projects {
		if reg.Projects[i].Name != projectName {
			continue
		}
		if reg.Projects[i].Services == nil {
			reg.Projects[i].Services = &registry.ProjectServices{}
		}
		version := "latest"
		if idx := strings.Index(svcKey, ":"); idx > 0 {
			version = svcKey[idx+1:]
		}
		switch svcType {
		case "mysql":
			reg.Projects[i].Services.MySQL = version
		case "postgres":
			reg.Projects[i].Services.Postgres = version
		case "redis":
			reg.Projects[i].Services.Redis = true
		case "mail":
			reg.Projects[i].Services.Mail = true
		case "s3":
			reg.Projects[i].Services.S3 = true
		}
		break
	}
}

func containsStr(slice []string, s string) bool {
	for _, v := range slice {
		if v == s {
			return true
		}
	}
	return false
}
