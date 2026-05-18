package automation

import (
	"testing"

	"github.com/prvious/pv/internal/config"
)

func TestContext_HoldsProjectConfig(t *testing.T) {
	cfg := &config.ProjectConfig{PHP: "8.4"}
	ctx := &Context{ProjectConfig: cfg}
	if ctx.ProjectConfig != cfg {
		t.Errorf("Context.ProjectConfig not preserved")
	}
}

func TestContext_NilProjectConfigOK(t *testing.T) {
	ctx := &Context{}
	if ctx.ProjectConfig.HasServices() {
		t.Errorf("nil ProjectConfig should report no services")
	}
	if ctx.ProjectConfig.HasAnyEnv() {
		t.Errorf("nil ProjectConfig should report no env")
	}
}
