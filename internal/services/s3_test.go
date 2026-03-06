package services

import "testing"

func TestS3Ports(t *testing.T) {
	s := &S3{}
	if got := s.Port("latest"); got != 9000 {
		t.Errorf("Port = %d, want 9000", got)
	}
	if got := s.ConsolePort("latest"); got != 9001 {
		t.Errorf("ConsolePort = %d, want 9001", got)
	}
}

func TestS3ImageName(t *testing.T) {
	s := &S3{}
	if got := s.ImageName("latest"); got != "rustfs/rustfs:latest" {
		t.Errorf("ImageName = %q, want %q", got, "rustfs/rustfs:latest")
	}
}

func TestS3EnvVars(t *testing.T) {
	s := &S3{}
	env := s.EnvVars("my_app", 9000)
	if env["AWS_ACCESS_KEY_ID"] != "minioadmin" {
		t.Errorf("AWS_ACCESS_KEY_ID = %q", env["AWS_ACCESS_KEY_ID"])
	}
	if env["AWS_BUCKET"] != "my_app" {
		t.Errorf("AWS_BUCKET = %q", env["AWS_BUCKET"])
	}
	if env["AWS_ENDPOINT"] != "http://127.0.0.1:9000" {
		t.Errorf("AWS_ENDPOINT = %q", env["AWS_ENDPOINT"])
	}
	if env["AWS_USE_PATH_STYLE_ENDPOINT"] != "true" {
		t.Errorf("AWS_USE_PATH_STYLE_ENDPOINT = %q", env["AWS_USE_PATH_STYLE_ENDPOINT"])
	}
}

func TestS3WebRoutes(t *testing.T) {
	s := &S3{}
	routes := s.WebRoutes()
	if len(routes) != 2 {
		t.Fatalf("WebRoutes len = %d, want 2", len(routes))
	}
	if routes[0].Subdomain != "s3" || routes[0].Port != 9001 {
		t.Errorf("route[0] = %+v, want {s3 9001}", routes[0])
	}
	if routes[1].Subdomain != "s3-api" || routes[1].Port != 9000 {
		t.Errorf("route[1] = %+v, want {s3-api 9000}", routes[1])
	}
}

func TestS3CreateOpts(t *testing.T) {
	s := &S3{}
	opts := s.CreateOpts("latest")
	if len(opts.Cmd) == 0 {
		t.Error("expected Cmd to be set for S3")
	}
	if opts.Ports[9001] != 9001 {
		t.Error("expected console port 9001 mapping")
	}
}

func TestS3Name(t *testing.T) {
	s := &S3{}
	if s.Name() != "s3" {
		t.Errorf("Name = %q, want %q", s.Name(), "s3")
	}
}
