DROP TRIGGER projects_primary_hostname_matches_hostname_update;

CREATE TRIGGER projects_primary_hostname_updates_hostname
AFTER UPDATE OF primary_hostname ON projects
WHEN EXISTS (
    SELECT 1
    FROM project_hostnames
    WHERE project_id = OLD.id
    AND is_primary = 1
)
BEGIN
    UPDATE project_hostnames
    SET hostname = NEW.primary_hostname
    WHERE project_id = OLD.id
    AND is_primary = 1;
END;
