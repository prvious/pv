ALTER TABLE projects ADD COLUMN project_slug TEXT;
ALTER TABLE projects
ADD COLUMN serves_http INTEGER NOT NULL DEFAULT 1 CHECK (serves_http IN (0, 1));

UPDATE projects SET project_slug = id WHERE project_slug IS NULL;

CREATE UNIQUE INDEX projects_project_slug_unique
ON projects(project_slug)
WHERE project_slug IS NOT NULL;

CREATE TRIGGER projects_project_slug_required_insert
BEFORE INSERT ON projects
WHEN NEW.project_slug IS NULL OR NEW.project_slug = ''
BEGIN
    SELECT RAISE(ABORT, 'project slug is required');
END;

CREATE TRIGGER projects_project_slug_immutable
BEFORE UPDATE OF project_slug ON projects
WHEN NEW.project_slug IS NULL
    OR NEW.project_slug = ''
    OR NEW.project_slug != OLD.project_slug
BEGIN
    SELECT RAISE(ABORT, 'project slug is immutable');
END;
