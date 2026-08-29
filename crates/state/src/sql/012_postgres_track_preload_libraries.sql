CREATE TABLE postgres_track_preload_libraries (
    track TEXT NOT NULL,
    library TEXT NOT NULL CHECK (
        library IN (
            'pg_stat_statements',
            'pg_cron',
            'timescaledb',
            'pg_duckdb'
        )
    ),
    PRIMARY KEY (track, library)
);
