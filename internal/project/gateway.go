package project

import "context"

type Route struct {
	ProjectPath string
	Hosts       []string
	TLS         bool
}

type GatewayAdapter interface {
	ApplyRoute(context.Context, Route) error
}

type Browser interface {
	Open(context.Context, string) error
}

func LinkGateway(ctx context.Context, adapter GatewayAdapter, route Route) error {
	return adapter.ApplyRoute(ctx, route)
}

func Open(ctx context.Context, browser Browser, contract Contract) error {
	return browser.Open(ctx, "https://"+contract.Hosts[0])
}
