package redis

import (
	"fmt"
	"strconv"
)

func TemplateVars(version string) map[string]string {
	const host = "127.0.0.1"
	port := PortFor(version)
	return map[string]string{
		"host":     host,
		"port":     strconv.Itoa(port),
		"password": "",
		"url":      fmt.Sprintf("redis://%s:%d", host, port),
	}
}
