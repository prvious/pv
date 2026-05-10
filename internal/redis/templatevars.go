package redis

import (
	"fmt"
	"strconv"
)

// TemplateVars returns the variables available inside a pv.yml
// `redis.env:` block. Redis is single-version with a fixed port, so
// no parameters are needed.
//
// Keys: host, port, password, url.
func TemplateVars() map[string]string {
	const host = "127.0.0.1"
	port := RedisPort
	return map[string]string{
		"host":     host,
		"port":     strconv.Itoa(port),
		"password": "",
		"url":      fmt.Sprintf("redis://%s:%d", host, port),
	}
}
