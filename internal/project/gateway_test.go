package project

import (
	"context"
	"testing"
)

func TestGatewayAndOpenUseAdapters(t *testing.T) {
	adapter := &routeAdapter{}
	route := Route{ProjectPath: "/app", Hosts: []string{"app.test"}, TLS: true}
	if err := LinkGateway(t.Context(), adapter, route); err != nil {
		t.Fatalf("LinkGateway returned error: %v", err)
	}
	if adapter.route.Hosts[0] != "app.test" {
		t.Fatalf("route = %#v", adapter.route)
	}
	browser := &fakeBrowser{}
	if err := Open(t.Context(), browser, Contract{Version: 1, PHP: "8.4", Hosts: []string{"app.test"}}); err != nil {
		t.Fatalf("Open returned error: %v", err)
	}
	if browser.url != "https://app.test" {
		t.Fatalf("url = %q, want https://app.test", browser.url)
	}
}

type routeAdapter struct {
	route Route
}

func (a *routeAdapter) ApplyRoute(_ context.Context, route Route) error {
	a.route = route
	return nil
}

type fakeBrowser struct {
	url string
}

func (b *fakeBrowser) Open(_ context.Context, url string) error {
	b.url = url
	return nil
}
