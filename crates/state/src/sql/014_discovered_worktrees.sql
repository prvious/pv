ALTER TABLE projects ADD COLUMN source_project_id TEXT REFERENCES projects(id) ON DELETE CASCADE;
ALTER TABLE projects ADD COLUMN worktree_id TEXT;

CREATE UNIQUE INDEX projects_discovered_worktree
ON projects(source_project_id, worktree_id)
WHERE source_project_id IS NOT NULL;

CREATE TABLE worktree_opt_outs (
    source_project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    worktree_id TEXT NOT NULL,
    PRIMARY KEY (source_project_id, worktree_id)
);
