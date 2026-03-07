# pv Service Management — Full Implementation Plan

## Overview

pv manages containerized services (MySQL, PostgreSQL, Redis, RustFS) via Colima + Docker Engine, controlled entirely through pv's CLI. Users never interact with Colima or Docker directly. The Go SDK (`github.com/docker/docker/client`) communicates with Docker Engine over the Unix socket — no Docker CLI binary needed.

## Decisions Summary

| Decision             | Answer                                                                       |
| -------------------- | ---------------------------------------------------------------------------- |
| Container runtime    | Colima (single binary, manages Lima VM + Docker Engine)                      |
| Docker interaction   | Go SDK via Unix socket, no Docker CLI binary                                 |
| Data on remove       | `pv service remove` = stop only, `pv service destroy` = delete data          |
| Database creation    | Auto-create database named after project during `pv link`                    |
| Service independence | Services run independently from PHP server (`pv stop` doesn't kill services) |
| Credentials          | root with no password (MySQL), postgres/no password (PostgreSQL)             |
| Colima lifecycle     | Tied to pv daemon — starts on login, always running                          |
| VM resources         | Default 2 CPU / 2GB RAM, ceiling 4 CPU / 8GB RAM                             |
| VM configurable      | No, fixed defaults for now                                                   |
| Day one services     | MySQL, PostgreSQL, Redis, RustFS                                             |
| Custom config files  | No, add later                                                                |
| Docker images        | Official Docker Hub images                                                   |
| Service env          | Auto-write to `.env` during `pv link` when database detected                 |

## Directory Structure

```
~/.pv/
├── bin/
│   └── colima                          # Colima binary
├── services/
│   ├── mysql/
│   │   ├── 8.0.32/
│   │   │   └── data/                   # MySQL data directory (mounted volume)
│   │   └── 8.0.45/
│   │       └── data/
│   ├── postgres/
│   │   └── 16/
│   │       └── data/
│   ├── redis/
│   │   └── 7.2/
│   │       └── data/
│   └── rustfs/
│       └── latest/
│           └── data/
└── data/
    └── registry.json                   # includes service bindings per project
```

## Registry Schema

```json
{
    "global_php": "8.4",
    "services": {
        "mysql:8.0.32": {
            "image": "mysql:8.0.32",
            "port": 33032,
            "status": "running",
            "container_id": "abc123..."
        },
        "mysql:8.0.45": {
            "image": "mysql:8.0.45",
            "port": 33045,
            "status": "running",
            "container_id": "def456..."
        },
        "redis": {
            "image": "redis:7.2",
            "port": 6379,
            "status": "running",
            "container_id": "ghi789..."
        },
        "rustfs": {
            "image": "rustfs/rustfs:latest",
            "port": 9000,
            "console_port": 9001,
            "status": "running",
            "container_id": "jkl012..."
        }
    },
    "projects": {
        "app-one": {
            "path": "/Users/clovis/code/app-one",
            "type": "laravel-octane",
            "php": "8.4",
            "services": {
                "mysql": "8.0.32",
                "redis": true,
                "rustfs": true
            },
            "databases": ["app_one"]
        },
        "app-two": {
            "path": "/Users/clovis/code/app-two",
            "type": "laravel",
            "php": "8.3",
            "services": {
                "postgres": "16",
                "redis": true
            },
            "databases": ["app_two"]
        }
    }
}
```

## Port Assignment

| Service    | Strategy                           | Examples                         |
| ---------- | ---------------------------------- | -------------------------------- |
| MySQL      | 33000 + patch version              | 8.0.32 → :33032, 8.0.45 → :33045 |
| PostgreSQL | 54000 + major version              | 16 → :54016, 17 → :54017         |
| Redis      | Fixed :6379                        | One shared instance              |
| RustFS     | Fixed :9000 (API), :9001 (console) | One shared instance              |

For MySQL, if two versions share the same patch number (unlikely but possible across major versions), fall back to sequential assignment from a base port.

---

## Task 1: Colima Binary Management

**1a: Download and install Colima**

- During `pv install`, download the Colima binary from GitHub releases
- Detect platform: `darwin/arm64` or `darwin/amd64`
- Place at `~/.pv/bin/colima`
- `chmod +x`
- Verify with `colima version`

**1b: Colima VM configuration**

- Default profile: `pv` (so it doesn't conflict with user's own Colima setup)
- Start command: `colima start --profile pv --cpu 2 --memory 2 --disk 60 --vm-type vz --mount-type virtiofs`
- `vz` = Apple Virtualization.framework (fastest on macOS)
- `virtiofs` = best file sharing performance
- 60GB disk should be plenty for images and data volumes

**1c: Integrate with pv daemon**

- When pv daemon starts (login), also start Colima: `colima start --profile pv`
- When pv daemon stops, also stop Colima: `colima stop --profile pv`
- Colima is invisible to the user — just plumbing that pv manages
- Store Colima status in registry so pv knows if it needs starting

---

## Task 2: Docker Engine Communication

**2a: Go SDK setup**

- Add `github.com/docker/docker/client` to go.mod
- Connect via Colima's Docker socket: `~/.colima/pv/docker.sock`
- Create a `internal/container/engine.go` with a client wrapper:
    - `PullImage(image string) error`
    - `CreateContainer(opts ContainerOpts) (string, error)`
    - `StartContainer(id string) error`
    - `StopContainer(id string) error`
    - `RemoveContainer(id string) error`
    - `IsRunning(id string) bool`
    - `Exec(id string, cmd []string) (string, error)` — for creating databases
    - `ListContainers(prefix string) ([]Container, error)`

**2b: Container naming convention**

- All pv containers prefixed: `pv-mysql-8.0.32`, `pv-redis-7.2`, `pv-rustfs`
- Labels on containers: `dev.prvious.pv=true`, `dev.prvious.pv.service=mysql`, `dev.prvious.pv.version=8.0.32`
- Labels allow pv to find its own containers without tracking IDs

**2c: Health checking**

- After starting a container, poll until the service is actually ready
- MySQL: attempt TCP connection to port, or exec `mysqladmin ping`
- PostgreSQL: exec `pg_isready`
- Redis: exec `redis-cli ping`, expect `PONG`
- RustFS: HTTP request to health endpoint
- Timeout after 30 seconds, report failure

---

## Task 3: Service Definitions

Create `internal/services/` with a definition per service type.

**3a: MySQL**

```go
type MySQLService struct {
    Version string  // "8.0.32"
    Port    int     // 33032
}
```

- Image: `mysql:[version]`
- Environment: `MYSQL_ALLOW_EMPTY_PASSWORD=yes`
- Volume: `~/.pv/services/mysql/[version]/data:/var/lib/mysql`
- Port: `<port>:3306`
- Health check: `mysqladmin ping -h 127.0.0.1`
- Database creation: `CREATE DATABASE IF NOT EXISTS <name>`
- Credentials: root, no password

**3b: PostgreSQL**

```go
type PostgresService struct {
    Version string  // "16"
    Port    int     // 54016
}
```

- Image: `postgres:[version]`
- Environment: `POSTGRES_HOST_AUTH_METHOD=trust`
- Volume: `~/.pv/services/postgres/[version]/data:/var/lib/postgresql/data`
- Port: `<port>:5432`
- Health check: `pg_isready`
- Database creation: `CREATE DATABASE <name>`
- Credentials: postgres, no password

**3c: Redis**

```go
type RedisService struct {
    Version string  // "7.2"
    Port    int     // 6379
}
```

- Image: `redis:[version]`
- Volume: `~/.pv/services/redis/[version]/data:/data`
- Port: `6379:6379`
- Health check: `redis-cli ping`
- No credentials, no per-project databases needed
- Shared across all projects via key prefixes

**3d: RustFS**

```go
type RustFSService struct {
    Port        int  // 9000
    ConsolePort int  // 9001
}
```

- Image: `rustfs/rustfs:latest`
- Environment: `RUSTFS_ROOT_USER=minioadmin`, `RUSTFS_ROOT_PASSWORD=minioadmin`
- Volume: `~/.pv/services/rustfs/latest/data:/data`
- Ports: `9000:9000` (S3 API), `9001:9001` (web console)
- Health check: HTTP GET to `:9000/minio/health/live` (S3-compatible endpoint)
- Command: `server /data --console-address ":9001"`
- Shared across all projects, each project gets its own bucket

---

## Task 4: `pv service add <service> [version]`

The main command for adding a service.

Flow:

1. Parse service name and optional version
2. If no version specified, resolve latest (MySQL → latest 8.x, PostgreSQL → latest, Redis → latest)
3. Check if this exact service+version already exists in registry → if so, print "already added" and exit
4. Ensure Colima is running (start if not)
5. Pull the Docker image (with spinner/progress)
6. Create data directory at `~/.pv/services/<service>/[version]/data/`
7. Create and start the container with appropriate config from Task 3
8. Wait for health check to pass
9. Update registry with container ID, port, status
10. Print connection details

Output:

```
$ pv service add mysql 8.0.32

  ✓ Pulled mysql:8.0.32
  ✓ MySQL 8.0.32 running on :33032

    Host:     127.0.0.1
    Port:     33032
    User:     root
    Password: (none)
```

---

## Task 5: `pv service remove <service>` and `pv service destroy <service>`

**`pv service remove mysql:8.0.32`**

1. Stop the container
2. Remove the container
3. Keep data directory intact at `~/.pv/services/mysql/8.0.32/data/`
4. Update registry (status → "stopped", clear container_id)
5. Check if any projects are bound to this service — warn but don't block

Output:

```
$ pv service remove mysql:8.0.32

  ✓ MySQL 8.0.32 stopped

    Data preserved at ~/.pv/services/mysql/8.0.32/data/
    Run 'pv service add mysql 8.0.32' to start it again.
```

**`pv service destroy mysql:8.0.32`**

1. Confirmation prompt: type "destroy" to confirm
2. Stop and remove container if running
3. Delete data directory: `rm -rf ~/.pv/services/mysql/8.0.32/`
4. Remove from registry entirely
5. Unbind from any projects that reference it

Output:

```
$ pv service destroy mysql:8.0.32

  This will permanently delete all MySQL 8.0.32 data.
  Type "destroy" to confirm: destroy

  ✓ MySQL 8.0.32 destroyed
  ⚠ Unbound from: app-one, legacy-app
```

---

## Task 6: `pv service list`

Display all services and their project bindings.

```
$ pv service list

  SERVICE          STATUS    PORT     PROJECTS
  mysql:8.0.32     running   :33032   app-one, legacy-app
  mysql:8.0.45     running   :33045   app-three
  postgres:16      running   :54016   app-six
  redis            running   :6379    (shared)
  rustfs           running   :9000    (shared)
```

If nothing added:

```
$ pv service list

  No services configured. Run 'pv service add mysql' to get started.
```

---

## Task 7: `pv service start` / `pv service stop`

Control individual or all services.

- `pv service start mysql:8.0.32` → start specific container
- `pv service stop mysql:8.0.32` → stop specific container
- `pv service start` → start all services in registry
- `pv service stop` → stop all services in registry

These do NOT affect Colima or the PHP server. Services are independent.

On start, reuse existing container if it exists (just stopped), otherwise create new one from the image with existing data volume.

---

## Task 8: `pv service env [service]`

Print or write environment variables for a service.

**When run standalone:**

```
$ pv service env mysql

  DB_CONNECTION=mysql
  DB_HOST=127.0.0.1
  DB_PORT=33032
  DB_DATABASE=app_one
  DB_USERNAME=root
  DB_PASSWORD=

  Write to .env? [Y/n]
```

Database name derived from current directory name (project name), with hyphens converted to underscores.

**Service-specific env mappings:**

MySQL:

```
DB_CONNECTION=mysql
DB_HOST=127.0.0.1
DB_PORT=<port>
DB_DATABASE=<project_name>
DB_USERNAME=root
DB_PASSWORD=
```

PostgreSQL:

```
DB_CONNECTION=pgsql
DB_HOST=127.0.0.1
DB_PORT=<port>
DB_DATABASE=<project_name>
DB_USERNAME=postgres
DB_PASSWORD=
```

Redis:

```
REDIS_HOST=127.0.0.1
REDIS_PORT=6379
REDIS_PASSWORD=null
```

RustFS:

```
AWS_ACCESS_KEY_ID=minioadmin
AWS_SECRET_ACCESS_KEY=minioadmin
AWS_DEFAULT_REGION=us-east-1
AWS_BUCKET=<project_name>
AWS_ENDPOINT=http://127.0.0.1:9000
AWS_USE_PATH_STYLE_ENDPOINT=true
```

**Writing to `.env`:**

- Read existing `.env` file
- Find matching keys, replace values in-place
- Keys not present in `.env` get appended
- Backup original to `.env.pv-backup` before writing

---

## Task 9: Auto-wiring During `pv link`

When `pv link` runs in a project, extend the existing detection logic:

1. Detect project type (existing — Laravel, PHP, etc.)
2. Resolve PHP version (existing — from composer.json)
3. **New: Detect required services**
    - Read `.env` for `DB_CONNECTION` value
    - `mysql` → check if a MySQL service is running, bind it
    - `pgsql` → check if a PostgreSQL service is running, bind it
    - Check for `REDIS_HOST` → bind Redis if running
    - Check for `AWS_ENDPOINT` with localhost → bind RustFS if running
4. **New: Auto-create database**
    - If MySQL or PostgreSQL is bound, exec into the container
    - `CREATE DATABASE IF NOT EXISTS <project_name>`
    - Project name = directory name, hyphens → underscores
5. **New: Offer to update `.env`**
    - If services were detected and bound, show what would change
    - Ask to write (Y/n)
    - Write changes using the same logic as `pv service env`

Output:

```
$ cd ~/code/my-app
$ pv link

  ✓ Detected Laravel + Octane
  ✓ PHP 8.4 (from composer.json)
  ✓ my-app.test ready

  Detected services:
    DB_CONNECTION=mysql → MySQL 8.0.32 on :33032
    REDIS_HOST → Redis on :6379

  ✓ Created database 'my_app' on MySQL 8.0.32
  Update .env with service connection details? [Y/n] y
  ✓ Updated .env (backup at .env.pv-backup)
```

If a required service isn't running:

```
  ⚠ DB_CONNECTION=mysql detected but no MySQL service running.
    Run: pv service add mysql
```

---

## Task 10: `pv service status`

Detailed status for a specific service:

```
$ pv service status mysql:8.0.32

  MySQL 8.0.32
    Status:     running
    Container:  pv-mysql-8.0.32
    Port:       :33032
    Uptime:     3 days, 14 hours
    Data:       ~/.pv/services/mysql/8.0.32/data/ (2.4 GB)
    Databases:  app_one, legacy_app, my_app
    Projects:   app-one, legacy-app, my-app
```

List databases by exec'ing `SHOW DATABASES` and filtering out system databases. Show disk usage of the data directory.

---

## Task 11: Colima Auto-Recovery

Since Colima is tied to the pv daemon via launchd:

- If Colima crashes or the VM dies, the daemon should detect this and restart it
- On daemon start, check if Colima VM is running: `colima status --profile pv`
- If not running, start it
- After Colima starts, check which service containers should be running (from registry) and start any that aren't
- This ensures services survive reboots: login → daemon starts → Colima starts → containers start

Add restart policy to containers: `RestartPolicy: always` so Docker Engine inside Colima auto-restarts containers if they crash within a running VM.

---

## Task 12: Integration with `pv doctor`

Add service health checks to `pv doctor`:

```
Services
  ✓ Colima VM running (2 CPU, 2GB RAM)
  ✓ Docker Engine reachable
  ✓ mysql:8.0.32 running on :33032
  ✓ redis running on :6379
  ✗ rustfs not running
    → Run: pv service start rustfs
```

Check:

- Colima VM is running
- Docker socket is reachable
- Each registered service container is running
- Each service passes its health check
- Ports are not conflicting with other processes

---

## Task 13: Integration with `pv uninstall`

When `pv uninstall` runs, add to the teardown sequence:

1. Stop all service containers
2. Remove all service containers
3. Stop Colima VM: `colima stop --profile pv`
4. Delete Colima profile: `colima delete --profile pv`
5. Data directories are nuked with `rm -rf ~/.pv/` (already in uninstall plan)

Prompt before destroying service data:

```
  You have service data that will be permanently deleted:
    MySQL 8.0.32:  2.4 GB (databases: app_one, legacy_app)
    PostgreSQL 16: 1.1 GB (databases: app_six)

  Type "uninstall" to confirm:
```

---

## Task 14: `pv service logs <service>`

Tail container logs for debugging:

- `pv service logs mysql` → `docker logs -f pv-mysql-8.0.32`
- `pv service logs redis` → `docker logs -f pv-redis-7.2`
- Via Go SDK: `client.ContainerLogs(ctx, containerID, types.ContainerLogsOptions{Follow: true})`

---

## Implementation Order

1. **Task 1** — Colima binary management (download, install, start/stop)
2. **Task 2** — Docker Engine communication via Go SDK
3. **Task 3** — Service definitions (MySQL, PostgreSQL, Redis, RustFS)
4. **Task 4** — `pv service add`
5. **Task 7** — `pv service start` / `pv service stop`
6. **Task 6** — `pv service list`
7. **Task 5** — `pv service remove` / `pv service destroy`
8. **Task 8** — `pv service env`
9. **Task 9** — Auto-wiring during `pv link`
10. **Task 10** — `pv service status`
11. **Task 14** — `pv service logs`
12. **Task 11** — Colima auto-recovery in daemon
13. **Task 12** — Integration with `pv doctor`
14. **Task 13** — Integration with `pv uninstall`

Core functionality (Tasks 1-7) first, then the smart integrations (8-9), then polish (10-14).
