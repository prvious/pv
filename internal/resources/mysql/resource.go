package mysql

import (
	"context"

	"github.com/prvious/pv/internal/control"
)

type Database interface {
	Create(context.Context, string) error
	Drop(context.Context, string) error
	List(context.Context) ([]string, error)
}

type Commands struct {
	DB Database
}

func Env(version string) map[string]string {
	return map[string]string{
		"DB_CONNECTION": "mysql",
		"DB_HOST":       "127.0.0.1",
		"DB_PORT":       "3306",
		"PV_MYSQL":      version,
	}
}

func Desired(version string) control.DesiredResource {
	return control.DesiredResource{Resource: control.ResourceMySQL, Version: version}
}

func (c Commands) Create(ctx context.Context, name string) error {
	return c.DB.Create(ctx, name)
}

func (c Commands) Drop(ctx context.Context, name string) error {
	return c.DB.Drop(ctx, name)
}

func (c Commands) List(ctx context.Context) ([]string, error) {
	return c.DB.List(ctx)
}
