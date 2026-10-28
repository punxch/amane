ALTER TABLE metadata ADD COLUMN field_sources TEXT;

CREATE TABLE source_snapshot (
    metadata_id INTEGER NOT NULL REFERENCES metadata (id) ON DELETE CASCADE,
    source_id TEXT NOT NULL,
    payload TEXT NOT NULL,
    fetched_at TEXT NOT NULL,
    PRIMARY KEY (metadata_id, source_id)
);

CREATE INDEX idx_source_snapshot_source ON source_snapshot (source_id);
