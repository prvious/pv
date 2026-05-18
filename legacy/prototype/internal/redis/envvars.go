package redis

import "strconv"

func EnvVars(version, projectName string) map[string]string {
	_ = projectName
	return map[string]string{
		"REDIS_HOST":     "127.0.0.1",
		"REDIS_PORT":     strconv.Itoa(PortFor(version)),
		"REDIS_PASSWORD": "null",
	}
}
