CREATE TABLE retired_resource_allocations (
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    resource_name TEXT NOT NULL,
    track TEXT NOT NULL,
    generated_name TEXT NOT NULL,
    retired_at TEXT NOT NULL,
    PRIMARY KEY (resource_name, track, generated_name)
);
