package services

import (
	"time"

	"github.com/prvious/pv/internal/binaries"
)

type RustFS struct{}

func (r *RustFS) Name() string        { return "s3" }
func (r *RustFS) DisplayName() string { return "S3 Storage (RustFS)" }

func (r *RustFS) Binary() binaries.Binary { return binaries.Rustfs }

// Args builds the rustfs server invocation. --console-enable is required;
// without it RustFS does not bind the console port.
// Verified against rustfs 1.0.0-alpha.93.
func (r *RustFS) Args(dataDir string) []string {
	return []string{
		"server", dataDir,
		"--address", ":9000",
		"--console-enable",
		"--console-address", ":9001",
	}
}

// Env sets the RustFS admin credentials. The correct env var names per
// `rustfs server --help` are ACCESS_KEY / SECRET_KEY (NOT ROOT_USER /
// ROOT_PASSWORD — those are invalid).
func (r *RustFS) Env() []string {
	return []string{
		"RUSTFS_ACCESS_KEY=rstfsadmin",
		"RUSTFS_SECRET_KEY=rstfsadmin",
	}
}

func (r *RustFS) Port() int        { return 9000 }
func (r *RustFS) ConsolePort() int { return 9001 }

func (r *RustFS) WebRoutes() []WebRoute {
	return []WebRoute{
		{Subdomain: "s3", Port: 9001},
		{Subdomain: "s3-api", Port: 9000},
	}
}

func (r *RustFS) EnvVars(projectName string) map[string]string {
	return map[string]string{
		"AWS_ACCESS_KEY_ID":           "rstfsadmin",
		"AWS_SECRET_ACCESS_KEY":       "rstfsadmin",
		"AWS_DEFAULT_REGION":          "us-east-1",
		"AWS_BUCKET":                  projectName,
		"AWS_ENDPOINT":                "http://127.0.0.1:9000",
		"AWS_USE_PATH_STYLE_ENDPOINT": "true",
	}
}

func (r *RustFS) ReadyCheck() ReadyCheck {
	return TCPReady(9000, 30*time.Second)
}

func init() {
	binaryRegistry["s3"] = &RustFS{}
}
