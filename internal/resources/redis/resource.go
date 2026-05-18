package redis

import "github.com/prvious/pv/internal/control"

func Desired(version string) control.DesiredResource {
	return control.DesiredResource{Resource: control.ResourceRedis, Version: version}
}

func Env(version string) map[string]string {
	return map[string]string{
		"REDIS_HOST": "127.0.0.1",
		"REDIS_PORT": "6379",
		"PV_REDIS":   version,
	}
}
