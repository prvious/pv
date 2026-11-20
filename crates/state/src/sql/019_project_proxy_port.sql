ALTER TABLE projects ADD COLUMN proxy_port INTEGER CHECK (proxy_port BETWEEN 1 AND 65535);
CREATE UNIQUE INDEX projects_proxy_port ON projects (proxy_port) WHERE proxy_port IS NOT NULL;
