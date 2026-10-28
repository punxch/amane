ALTER TABLE metadata ADD COLUMN note TEXT;

CREATE TABLE metadata_lock (
    metadata_id INTEGER NOT NULL REFERENCES metadata (id) ON DELETE CASCADE,
    field TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (metadata_id, field)
);

CREATE TABLE user_tag (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL
);

CREATE TABLE metadata_tag (
    metadata_id INTEGER NOT NULL REFERENCES metadata (id) ON DELETE CASCADE,
    tag_id INTEGER NOT NULL REFERENCES user_tag (id) ON DELETE CASCADE,
    PRIMARY KEY (metadata_id, tag_id)
);

CREATE INDEX idx_metadata_tag_tag ON metadata_tag (tag_id);
