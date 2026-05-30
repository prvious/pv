ALTER TABLE managed_resource_tracks
ADD COLUMN env_json TEXT NOT NULL DEFAULT '{}';

CREATE TABLE project_managed_resources (
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    resource_name TEXT NOT NULL,
    track TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (project_id, resource_name),
    FOREIGN KEY (resource_name, track)
        REFERENCES managed_resource_tracks(resource_name, track)
);

CREATE TRIGGER project_managed_resources_usage_insert
AFTER INSERT ON project_managed_resources
BEGIN
    UPDATE managed_resource_tracks
    SET usage_count = (
        SELECT COUNT(*)
        FROM project_managed_resources
        WHERE resource_name = NEW.resource_name
        AND track = NEW.track
    )
    WHERE resource_name = NEW.resource_name
    AND track = NEW.track;
END;

CREATE TRIGGER project_managed_resources_usage_update
AFTER UPDATE OF resource_name, track ON project_managed_resources
BEGIN
    UPDATE managed_resource_tracks
    SET usage_count = (
        SELECT COUNT(*)
        FROM project_managed_resources
        WHERE resource_name = OLD.resource_name
        AND track = OLD.track
    )
    WHERE resource_name = OLD.resource_name
    AND track = OLD.track;

    UPDATE managed_resource_tracks
    SET usage_count = (
        SELECT COUNT(*)
        FROM project_managed_resources
        WHERE resource_name = NEW.resource_name
        AND track = NEW.track
    )
    WHERE resource_name = NEW.resource_name
    AND track = NEW.track;
END;

CREATE TRIGGER project_managed_resources_usage_delete
AFTER DELETE ON project_managed_resources
BEGIN
    UPDATE managed_resource_tracks
    SET usage_count = (
        SELECT COUNT(*)
        FROM project_managed_resources
        WHERE resource_name = OLD.resource_name
        AND track = OLD.track
    )
    WHERE resource_name = OLD.resource_name
    AND track = OLD.track;
END;

CREATE TABLE project_env_observed_warnings (
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    warning_kind TEXT NOT NULL,
    message TEXT NOT NULL,
    observed_at TEXT NOT NULL,
    PRIMARY KEY (project_id, warning_kind, message)
);
