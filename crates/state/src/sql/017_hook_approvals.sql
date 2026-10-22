CREATE TABLE hook_approvals (
    folder TEXT NOT NULL,
    command TEXT NOT NULL,
    approved_at TEXT NOT NULL,
    PRIMARY KEY (folder, command)
);
