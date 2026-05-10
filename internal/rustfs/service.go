package rustfs

import (
	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/caddy"
	rustfsproc "github.com/prvious/pv/internal/rustfs/proc"
	"github.com/prvious/pv/internal/supervisor"
)

const (
	displayName = "S3 Storage (RustFS)"
	serviceKey  = "s3" // registry key + binding key — DO NOT change without a registry migration
	port        = 9000
	consolePort = 9001
)

// Binary returns the binaries.Binary descriptor for rustfs.
// Delegates to the leaf proc package so that internal/server can import
// proc directly without creating an import cycle through this package.
func Binary() binaries.Binary { return rustfsproc.Binary() }

func Port() int           { return port }
func ConsolePort() int    { return consolePort }
func DisplayName() string { return displayName }
func ServiceKey() string  { return serviceKey }

func WebRoutes() []caddy.WebRoute {
	return []caddy.WebRoute{
		{Subdomain: "s3", Port: consolePort},
		{Subdomain: "s3-api", Port: port},
	}
}

func EnvVars(projectName string) map[string]string {
	return map[string]string{
		"AWS_ACCESS_KEY_ID":           "rstfsadmin",
		"AWS_SECRET_ACCESS_KEY":       "rstfsadmin",
		"AWS_DEFAULT_REGION":          "us-east-1",
		"AWS_BUCKET":                  projectName,
		"AWS_ENDPOINT":                "http://127.0.0.1:9000",
		"AWS_USE_PATH_STYLE_ENDPOINT": "true",
	}
}

// BuildSupervisorProcess returns the supervisor.Process for rustfs.
// Delegates to proc.BuildSupervisorProcess so the build logic is defined
// once in the leaf package.
func BuildSupervisorProcess() (supervisor.Process, error) {
	return rustfsproc.BuildSupervisorProcess()
}
