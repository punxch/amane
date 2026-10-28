CREATE TABLE resource (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    url TEXT NOT NULL UNIQUE,
    file_name TEXT NOT NULL,
    content_type TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    created_at TEXT NOT NULL
);

ALTER TABLE metadata ADD COLUMN poster_resource_id INTEGER REFERENCES resource (id) ON DELETE SET NULL;
