CREATE TABLE schedule (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL,
    payload TEXT NOT NULL,
    cron TEXT NOT NULL,
    timezone TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    last_fired_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- 触发来源; 定时任务被删除后, 历史任务保留, 来源置空.
ALTER TABLE task ADD COLUMN schedule_id INTEGER REFERENCES schedule (id) ON DELETE SET NULL;
