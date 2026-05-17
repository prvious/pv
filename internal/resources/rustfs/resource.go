package rustfs

import "github.com/prvious/pv/internal/control"

type Credentials struct {
	AccessKey string
	SecretKey string
}

func Desired(version string) control.DesiredResource {
	return control.DesiredResource{Resource: control.ResourceRustFS, Version: version}
}

func Env(version string, credentials Credentials) map[string]string {
	return map[string]string{
		"AWS_ACCESS_KEY_ID":     credentials.AccessKey,
		"AWS_SECRET_ACCESS_KEY": credentials.SecretKey,
		"AWS_ENDPOINT_URL":      "http://127.0.0.1:9000",
		"PV_RUSTFS":             version,
	}
}

func RedactedStatus(credentials Credentials) map[string]string {
	status := map[string]string{
		"AWS_ENDPOINT_URL": "http://127.0.0.1:9000",
	}
	if credentials.AccessKey != "" {
		status["AWS_ACCESS_KEY_ID"] = "<redacted>"
	}
	if credentials.SecretKey != "" {
		status["AWS_SECRET_ACCESS_KEY"] = "<redacted>"
	}
	return status
}
