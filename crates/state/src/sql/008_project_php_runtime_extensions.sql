ALTER TABLE projects ADD COLUMN desired_php_requested_extensions_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE projects ADD COLUMN desired_php_loaded_extensions_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE projects ADD COLUMN desired_php_ignored_extensions_json TEXT NOT NULL DEFAULT '[]';
