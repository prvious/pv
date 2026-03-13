package steps

import (
	"github.com/prvious/pv/internal/automation"
	"github.com/prvious/pv/internal/commands/php"
	"github.com/prvious/pv/internal/phpenv"
)

// InstallPHPStep installs a non-global PHP version if it is not already present.
type InstallPHPStep struct{}

var _ automation.Step = (*InstallPHPStep)(nil)

func (s *InstallPHPStep) Label() string  { return "Install PHP version" }
func (s *InstallPHPStep) Gate() string   { return "install_php_version" }
func (s *InstallPHPStep) Critical() bool { return true }

func (s *InstallPHPStep) ShouldRun(ctx *automation.Context) bool {
	return ctx.PHPVersion != "" &&
		ctx.PHPVersion != ctx.GlobalPHP &&
		!phpenv.IsInstalled(ctx.PHPVersion)
}

func (s *InstallPHPStep) Run(ctx *automation.Context) (string, error) {
	if err := php.RunInstall([]string{ctx.PHPVersion}); err != nil {
		return "", err
	}
	return "PHP " + ctx.PHPVersion + " installed", nil
}
