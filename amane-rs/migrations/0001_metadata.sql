CREATE TABLE metadata (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    number TEXT NOT NULL UNIQUE,
    title TEXT,
    content_type TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX idx_metadata_updated_at ON metadata (updated_at DESC, id DESC);
