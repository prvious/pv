CREATE TABLE job_diagnostic_outcomes (
    sequence INTEGER PRIMARY KEY,
    job_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    subject_kind TEXT NOT NULL,
    subject_id TEXT NOT NULL,
    outcome TEXT NOT NULL CHECK (outcome IN ('failure', 'success')),
    UNIQUE (job_id, subject_kind, subject_id, outcome)
);

CREATE INDEX job_diagnostic_outcomes_subject
ON job_diagnostic_outcomes(subject_kind, subject_id, outcome);
