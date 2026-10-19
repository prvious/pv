ALTER TABLE jobs ADD COLUMN finished_sequence INTEGER;

WITH completed AS (
    SELECT id, ROW_NUMBER() OVER (ORDER BY finished_at, id) AS sequence
    FROM jobs WHERE status != 'running'
)
UPDATE jobs SET finished_sequence = (
    SELECT sequence FROM completed WHERE completed.id = jobs.id
) WHERE status != 'running';

CREATE TABLE job_counters (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    last_job_number INTEGER NOT NULL,
    last_finished_sequence INTEGER NOT NULL
);

INSERT INTO job_counters
SELECT 1,
       COALESCE(MAX(CASE WHEN id GLOB 'job_[0-9]*' THEN CAST(SUBSTR(id, 5) AS INTEGER) END), 0),
       COALESCE(MAX(finished_sequence), 0)
FROM jobs;
