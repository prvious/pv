package status

import (
	"strings"
	"testing"
)

func TestRenderFiltersTargetedViewsAndRedactsSecrets(t *testing.T) {
	output, err := Render([]Entry{
		{
			View:       ViewResource,
			Name:       "rustfs",
			Desired:    "1.0.0",
			Observed:   "running",
			State:      StateHealthy,
			LogPath:    "/tmp/rustfs.log",
			NextAction: "none",
			Values: map[string]string{
				"AWS_SECRET_ACCESS_KEY": "secret",
				"AWS_ENDPOINT_URL":      "http://127.0.0.1:9000",
			},
		},
		{View: ViewProject, Name: "app", State: StatePartial},
	}, ViewResource)

	if err != nil {
		t.Fatalf("Render returned error: %v", err)
	}
	for _, want := range []string{
		"resource rustfs: healthy",
		"desired: 1.0.0",
		"AWS_SECRET_ACCESS_KEY=<redacted>",
	} {
		if !strings.Contains(output, want) {
			t.Fatalf("output missing %q:\n%s", want, output)
		}
	}
	if strings.Contains(output, "project app") || strings.Contains(output, "secret\n") {
		t.Fatalf("output included wrong view or secret:\n%s", output)
	}
}

func TestValidateViewRejectsUnknownView(t *testing.T) {
	if err := ValidateView("daemon"); err == nil {
		t.Fatal("ValidateView returned nil error")
	}
}

func TestRenderSupportsAllNormalizedStates(t *testing.T) {
	for _, state := range []State{
		StateHealthy,
		StateStopped,
		StateMissingInstall,
		StateBlocked,
		StateCrashed,
		StateFailed,
		StatePartial,
		StateUnknown,
	} {
		output, err := Render([]Entry{{View: ViewGateway, Name: string(state), State: state}}, "")
		if err != nil {
			t.Fatalf("Render(%s) returned error: %v", state, err)
		}
		if !strings.Contains(output, string(state)) {
			t.Fatalf("output missing %s:\n%s", state, output)
		}
	}
}
