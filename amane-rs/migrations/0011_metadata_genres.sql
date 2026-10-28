ALTER TABLE metadata ADD COLUMN genres TEXT;

CREATE INDEX idx_metadata_genres ON metadata (genres);
