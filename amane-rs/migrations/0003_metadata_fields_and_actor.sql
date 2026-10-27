ALTER TABLE metadata ADD COLUMN plot TEXT;
ALTER TABLE metadata ADD COLUMN release_date TEXT;
ALTER TABLE metadata ADD COLUMN runtime_minutes INTEGER;
ALTER TABLE metadata ADD COLUMN maker TEXT;
ALTER TABLE metadata ADD COLUMN label TEXT;
ALTER TABLE metadata ADD COLUMN series TEXT;
ALTER TABLE metadata ADD COLUMN poster_url TEXT;

CREATE TABLE actor (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE metadata_actor (
    metadata_id INTEGER NOT NULL REFERENCES metadata (id) ON DELETE CASCADE,
    actor_id INTEGER NOT NULL REFERENCES actor (id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL,
    PRIMARY KEY (metadata_id, actor_id)
);

CREATE INDEX idx_metadata_actor_actor ON metadata_actor (actor_id);
