CREATE TABLE global_php_default_track (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    track TEXT NOT NULL CHECK (
        track NOT IN ('', '.', '..', 'latest')
        AND track NOT GLOB '*[^A-Za-z0-9._-]*'
    ),
    updated_at TEXT NOT NULL
);
