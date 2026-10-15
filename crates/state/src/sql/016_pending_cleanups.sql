CREATE TABLE pending_cleanups (
    resource_name TEXT NOT NULL,
    track TEXT NOT NULL,
    generated_name TEXT NOT NULL,
    project_slug TEXT NOT NULL,
    last_error TEXT,
    created_at TEXT NOT NULL,
    PRIMARY KEY (resource_name, track, generated_name)
);
