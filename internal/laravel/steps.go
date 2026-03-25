package laravel

import (
	"fmt"
	"os"
	"path/filepath"

	"github.com/prvious/pv/internal/automation"
	"github.com/prvious/pv/internal/services"
)

// isLaravel returns true if the project type is Laravel or Laravel with Octane.
func isLaravel(projectType string) bool {
	return projectType == "laravel" || projectType == "laravel-octane"
}

// --- CopyEnvStep ---

// CopyEnvStep copies .env.example to .env and loads it into ctx.Env.
type CopyEnvStep struct{}

var _ automation.Step = (*CopyEnvStep)(nil)

func (s *CopyEnvStep) Label() string  { return "Copy .env" }
func (s *CopyEnvStep) Gate() string   { return "copy_env" }
func (s *CopyEnvStep) Critical() bool { return false }

func (s *CopyEnvStep) ShouldRun(ctx *automation.Context) bool {
	if !isLaravel(ctx.ProjectType) {
		return false
	}
	if !HasEnvExample(ctx.ProjectPath) {
		return false
	}
	return !HasEnvFile(ctx.ProjectPath)
}

func (s *CopyEnvStep) Run(ctx *automation.Context) (string, error) {
	src := filepath.Join(ctx.ProjectPath, ".env.example")
	dst := filepath.Join(ctx.ProjectPath, ".env")
	data, err := os.ReadFile(src)
	if err != nil {
		return "", fmt.Errorf("read .env.example: %w", err)
	}
	if err := os.WriteFile(dst, data, 0644); err != nil {
		return "", fmt.Errorf("write .env: %w", err)
	}
	env, err := services.ReadDotEnv(dst)
	if err != nil {
		return "", fmt.Errorf("parse .env: %w", err)
	}
	ctx.Env = env
	return "copied .env.example → .env", nil
}

// --- GenerateKeyStep ---

// GenerateKeyStep runs artisan key:generate if APP_KEY is empty.
type GenerateKeyStep struct{}

var _ automation.Step = (*GenerateKeyStep)(nil)

func (s *GenerateKeyStep) Label() string  { return "Generate application key" }
func (s *GenerateKeyStep) Gate() string   { return "generate_key" }
func (s *GenerateKeyStep) Critical() bool { return false }

func (s *GenerateKeyStep) ShouldRun(ctx *automation.Context) bool {
	if !isLaravel(ctx.ProjectType) {
		return false
	}
	if !HasEnvFile(ctx.ProjectPath) {
		return false
	}
	return ReadAppKey(ctx.ProjectPath) == ""
}

func (s *GenerateKeyStep) Run(ctx *automation.Context) (string, error) {
	if err := KeyGenerate(ctx.ProjectPath, "php"); err != nil {
		return "", fmt.Errorf("artisan key:generate: %w", err)
	}
	return "application key generated", nil
}

// --- SetAppURLStep ---

// SetAppURLStep sets APP_URL in .env to https://{name}.{tld}.
type SetAppURLStep struct{}

var _ automation.Step = (*SetAppURLStep)(nil)

func (s *SetAppURLStep) Label() string  { return "Set APP_URL" }
func (s *SetAppURLStep) Gate() string   { return "set_app_url" }
func (s *SetAppURLStep) Critical() bool { return false }

func (s *SetAppURLStep) ShouldRun(ctx *automation.Context) bool {
	return isLaravel(ctx.ProjectType) && HasEnvFile(ctx.ProjectPath)
}

func (s *SetAppURLStep) Run(ctx *automation.Context) (string, error) {
	tld := ctx.TLD
	if tld == "" {
		tld = "test"
	}
	appURL := fmt.Sprintf("https://%s.%s", ctx.ProjectName, tld)
	envPath := filepath.Join(ctx.ProjectPath, ".env")
	vars := map[string]string{"APP_URL": appURL}
	if err := services.MergeDotEnv(envPath, "", vars); err != nil {
		return "", fmt.Errorf("set APP_URL: %w", err)
	}
	return appURL, nil
}

// --- InstallOctaneStep ---

// InstallOctaneStep runs artisan octane:install if octane is in composer.json
// but the worker file is missing.
type InstallOctaneStep struct{}

var _ automation.Step = (*InstallOctaneStep)(nil)

func (s *InstallOctaneStep) Label() string  { return "Install Octane" }
func (s *InstallOctaneStep) Gate() string   { return "install_octane" }
func (s *InstallOctaneStep) Critical() bool { return false }

func (s *InstallOctaneStep) ShouldRun(ctx *automation.Context) bool {
	if !isLaravel(ctx.ProjectType) {
		return false
	}
	return HasOctanePackage(ctx.ProjectPath) && !HasOctaneWorker(ctx.ProjectPath)
}

func (s *InstallOctaneStep) Run(ctx *automation.Context) (string, error) {
	if err := OctaneInstall(ctx.ProjectPath, "php"); err != nil {
		return "", fmt.Errorf("artisan octane:install: %w", err)
	}

	// Re-detect project type after Octane installation.
	if HasOctaneWorker(ctx.ProjectPath) && ctx.ProjectType != "laravel-octane" {
		ctx.ProjectType = "laravel-octane"
		// Update registry to reflect the new type.
		for i := range ctx.Registry.Projects {
			if ctx.Registry.Projects[i].Name == ctx.ProjectName {
				ctx.Registry.Projects[i].Type = "laravel-octane"
				break
			}
		}
	}

	return "Octane installed with FrankenPHP", nil
}

// --- ComposerInstallStep ---

// ComposerInstallStep runs composer install if vendor/ is missing.
type ComposerInstallStep struct{}

var _ automation.Step = (*ComposerInstallStep)(nil)

func (s *ComposerInstallStep) Label() string  { return "Install Composer dependencies" }
func (s *ComposerInstallStep) Gate() string   { return "composer_install" }
func (s *ComposerInstallStep) Critical() bool { return false }

func (s *ComposerInstallStep) ShouldRun(ctx *automation.Context) bool {
	if !isLaravel(ctx.ProjectType) {
		return false
	}
	if !HasComposerJSON(ctx.ProjectPath) {
		return false
	}
	return !HasVendorDir(ctx.ProjectPath)
}

func (s *ComposerInstallStep) Run(ctx *automation.Context) (string, error) {
	out, err := ComposerInstall(ctx.ProjectPath)
	if err != nil {
		return "", fmt.Errorf("composer install: %w", err)
	}
	return out, nil
}

// --- DetectServicesStep ---

// DetectServicesStep merges smart env vars for bound services into .env.
type DetectServicesStep struct{}

var _ automation.Step = (*DetectServicesStep)(nil)

func (s *DetectServicesStep) Label() string  { return "Configure service environment" }
func (s *DetectServicesStep) Gate() string   { return "update_env_on_service" }
func (s *DetectServicesStep) Critical() bool { return false }

func (s *DetectServicesStep) ShouldRun(ctx *automation.Context) bool {
	if !isLaravel(ctx.ProjectType) {
		return false
	}
	if ctx.Registry == nil {
		return false
	}
	proj := ctx.Registry.Find(ctx.ProjectName)
	if proj == nil || proj.Services == nil {
		return false
	}
	// Run if any service is bound.
	svc := proj.Services
	return svc.Redis || svc.S3 || svc.Mail || svc.MySQL != "" || svc.Postgres != ""
}

func (s *DetectServicesStep) Run(ctx *automation.Context) (string, error) {
	proj := ctx.Registry.Find(ctx.ProjectName)
	if proj == nil || proj.Services == nil {
		return "no services bound", nil
	}
	vars := SmartEnvVars(proj.Services)
	if len(vars) == 0 {
		return "no env vars to set", nil
	}
	envPath := filepath.Join(ctx.ProjectPath, ".env")
	if err := services.MergeDotEnv(envPath, "", vars); err != nil {
		return "", fmt.Errorf("merge service env: %w", err)
	}
	return fmt.Sprintf("set %d service env vars", len(vars)), nil
}

// --- CreateDatabaseStep ---

// CreateDatabaseStep resolves the database name and records it in the registry.
type CreateDatabaseStep struct{}

var _ automation.Step = (*CreateDatabaseStep)(nil)

func (s *CreateDatabaseStep) Label() string  { return "Create database" }
func (s *CreateDatabaseStep) Gate() string   { return "create_database" }
func (s *CreateDatabaseStep) Critical() bool { return false }

func (s *CreateDatabaseStep) ShouldRun(ctx *automation.Context) bool {
	if !isLaravel(ctx.ProjectType) {
		return false
	}
	if ctx.Registry == nil {
		return false
	}
	proj := ctx.Registry.Find(ctx.ProjectName)
	if proj == nil || proj.Services == nil {
		return false
	}
	// Need a database service bound.
	return proj.Services.MySQL != "" || proj.Services.Postgres != ""
}

func (s *CreateDatabaseStep) Run(ctx *automation.Context) (string, error) {
	dbName := ResolveDatabaseName(ctx.ProjectPath, ctx.ProjectName)

	// Record in registry. Use index-based access so mutations persist
	// in the slice (Registry.Find returns a copy via range variable).
	for i := range ctx.Registry.Projects {
		if ctx.Registry.Projects[i].Name != ctx.ProjectName {
			continue
		}
		proj := &ctx.Registry.Projects[i]
		found := false
		for _, db := range proj.Databases {
			if db == dbName {
				found = true
				break
			}
		}
		if !found {
			proj.Databases = append(proj.Databases, dbName)
		}
		break
	}

	ctx.DBCreated = true
	return dbName, nil
}

// --- RunMigrationsStep ---

// RunMigrationsStep runs artisan migrate if a database was created in this pipeline.
type RunMigrationsStep struct{}

var _ automation.Step = (*RunMigrationsStep)(nil)

func (s *RunMigrationsStep) Label() string  { return "Run migrations" }
func (s *RunMigrationsStep) Gate() string   { return "run_migrations" }
func (s *RunMigrationsStep) Critical() bool { return false }

func (s *RunMigrationsStep) ShouldRun(ctx *automation.Context) bool {
	if !isLaravel(ctx.ProjectType) {
		return false
	}
	return ctx.DBCreated
}

func (s *RunMigrationsStep) Run(ctx *automation.Context) (string, error) {
	out, err := Migrate(ctx.ProjectPath, "php")
	if err != nil {
		return "", fmt.Errorf("artisan migrate: %w", err)
	}
	return out, nil
}
