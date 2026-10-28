ALTER TABLE metadata ADD COLUMN cover_url TEXT;

ALTER TABLE metadata ADD COLUMN cover_resource_id INTEGER REFERENCES resource (id) ON DELETE SET NULL;
