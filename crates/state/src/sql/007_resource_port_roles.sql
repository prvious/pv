CREATE TABLE resource_ports (
    resource_name TEXT NOT NULL,
    track TEXT NOT NULL,
    port_name TEXT NOT NULL,
    port INTEGER NOT NULL UNIQUE,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (resource_name, track, port_name)
);
