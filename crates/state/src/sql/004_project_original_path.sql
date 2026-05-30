ALTER TABLE projects ADD COLUMN original_path TEXT;
UPDATE projects SET original_path = path WHERE original_path IS NULL;
