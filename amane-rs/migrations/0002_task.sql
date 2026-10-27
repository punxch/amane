CREATE TABLE task (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL,
    payload TEXT NOT NULL,
    status TEXT NOT NULL,
    priority INTEGER NOT NULL DEFAULT 0,
    attempts INTEGER NOT NULL DEFAULT 0,
    error TEXT,
    retry_of INTEGER REFERENCES task (id) ON DELETE SET NULL,
    created_at TEXT NOT NULL,
    started_at TEXT,
    finished_at TEXT
);

CREATE INDEX idx_task_status ON task (status, priority DESC, id);

CREATE TABLE task_link (
    parent_id INTEGER NOT NULL REFERENCES task (id) ON DELETE CASCADE,
    child_id INTEGER NOT NULL REFERENCES task (id) ON DELETE CASCADE,
    PRIMARY KEY (parent_id, child_id)
);
