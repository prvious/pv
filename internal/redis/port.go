// Package redis owns the lifecycle of the native redis binary managed by
// pv. Mirrors internal/postgres/ and internal/mysql/ but flat:
// single-version, no per-version map. State at ~/.pv/redis/ and
// ~/.pv/data/redis/.
package redis

// RedisPort is the TCP port pv binds redis-server to. Constant 6379 —
// the upstream default and the value every Laravel app expects out of
// the box. Single-version means there's no collision risk.
const RedisPort = 6379

// PortFor returns the TCP port redis-server should bind to.
// Kept as a function (not just exposing the const) for parallel API
// shape with mysql.PortFor / postgres.PortFor — callers don't have to
// branch on which package they're talking to.
func PortFor() int { return RedisPort }
