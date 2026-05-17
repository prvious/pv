package postgres

import (
	"context"
	"testing"

	"github.com/prvious/pv/internal/control"
)

func TestPostgresResourceIsExplicit(t *testing.T) {
	desired := Desired("18.0")
	if desired.Resource != control.ResourcePostgres {
		t.Fatalf("resource = %q", desired.Resource)
	}
	if Env("18.0")["DB_CONNECTION"] != "pgsql" {
		t.Fatalf("env = %#v", Env("18.0"))
	}
	db := &fakeDB{}
	commands := Commands{DB: db}
	if err := commands.Create(t.Context(), "app"); err != nil {
		t.Fatalf("Create returned error: %v", err)
	}
	if db.created != "app" {
		t.Fatalf("created = %q, want app", db.created)
	}
}

type fakeDB struct {
	created string
}

func (db *fakeDB) Create(_ context.Context, name string) error {
	db.created = name
	return nil
}

func (db *fakeDB) Drop(context.Context, string) error {
	return nil
}

func (db *fakeDB) List(context.Context) ([]string, error) {
	return nil, nil
}
