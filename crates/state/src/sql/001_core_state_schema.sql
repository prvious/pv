CREATE TABLE projects (
    id TEXT PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,
    primary_hostname TEXT NOT NULL UNIQUE,
    config_path TEXT,
    desired_php_track TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE project_hostnames (
    hostname TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    is_primary INTEGER NOT NULL CHECK (is_primary IN (0, 1)),
    created_at TEXT NOT NULL
);

CREATE UNIQUE INDEX project_hostnames_one_primary_per_project
ON project_hostnames(project_id)
WHERE is_primary = 1;

CREATE TRIGGER project_hostnames_primary_matches_project_insert
BEFORE INSERT ON project_hostnames
WHEN NEW.is_primary = 1
AND (
    SELECT primary_hostname
    FROM projects
    WHERE id = NEW.project_id
) != NEW.hostname
BEGIN
    SELECT RAISE(ABORT, 'primary hostname must match project primary_hostname');
END;

CREATE TRIGGER project_hostnames_primary_matches_project_update
BEFORE UPDATE OF hostname, project_id, is_primary ON project_hostnames
WHEN NEW.is_primary = 1
AND (
    SELECT primary_hostname
    FROM projects
    WHERE id = NEW.project_id
) != NEW.hostname
BEGIN
    SELECT RAISE(ABORT, 'primary hostname must match project primary_hostname');
END;

CREATE TRIGGER projects_primary_hostname_matches_hostname_update
BEFORE UPDATE OF primary_hostname ON projects
WHEN EXISTS (
    SELECT 1
    FROM project_hostnames
    WHERE project_id = OLD.id
    AND is_primary = 1
)
AND NOT EXISTS (
    SELECT 1
    FROM project_hostnames
    WHERE project_id = OLD.id
    AND is_primary = 1
    AND hostname = NEW.primary_hostname
)
BEGIN
    SELECT RAISE(ABORT, 'project primary_hostname must match primary project_hostname row');
END;

CREATE TRIGGER project_hostnames_primary_delete_requires_project_delete
BEFORE DELETE ON project_hostnames
WHEN OLD.is_primary = 1
AND EXISTS (
    SELECT 1
    FROM projects
    WHERE id = OLD.project_id
)
BEGIN
    SELECT RAISE(ABORT, 'primary project_hostname rows are removed with their project');
END;

CREATE TABLE managed_resource_tracks (
    resource_name TEXT NOT NULL,
    track TEXT NOT NULL,
    desired_state TEXT NOT NULL,
    installed_version TEXT,
    current_artifact_path TEXT,
    usage_count INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (resource_name, track)
);

CREATE TABLE resource_allocations (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    resource_name TEXT NOT NULL,
    track TEXT NOT NULL,
    allocation_name TEXT NOT NULL,
    generated_name TEXT NOT NULL,
    env_json TEXT NOT NULL DEFAULT '{}',
    status TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (project_id, resource_name, allocation_name),
    UNIQUE (resource_name, track, generated_name)
);

CREATE TABLE ports (
    name TEXT PRIMARY KEY,
    port INTEGER NOT NULL UNIQUE,
    owner_kind TEXT NOT NULL,
    resource_name TEXT,
    track TEXT,
    updated_at TEXT NOT NULL
);

CREATE TABLE observed_states (
    subject_kind TEXT NOT NULL,
    subject_id TEXT NOT NULL,
    status TEXT NOT NULL,
    message TEXT,
    observed_at TEXT NOT NULL,
    PRIMARY KEY (subject_kind, subject_id)
);

CREATE TABLE jobs (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    scope TEXT NOT NULL,
    status TEXT NOT NULL,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    summary TEXT,
    error TEXT
);
