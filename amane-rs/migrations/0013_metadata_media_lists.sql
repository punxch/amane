-- 图片与预告改存列表, 剧照、评分、外部标识与原站地址按站点分组.
--
-- SQLite 删列只能重建表, 而重建必须先关外键: 子表 (演员关联、锁、标签、快照) 都以
-- ON DELETE CASCADE 指向这里, 外键开着时 DROP TABLE 会先做一次隐式 DELETE, 把子表的行
-- 一并删掉. 外键由 `Database::migrate` 在跑迁移前关闭、跑完恢复 —— sqlx 的 SQLite 迁移
-- 总是包在事务里, 而事务内的 `PRAGMA foreign_keys` 是空操作, 写在这里不会生效.

CREATE TABLE metadata_rebuilt (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    number TEXT NOT NULL UNIQUE,
    title TEXT,
    plot TEXT,
    release_date TEXT,
    runtime_minutes INTEGER,
    maker TEXT,
    label TEXT,
    series TEXT,
    poster_urls TEXT,
    thumb_urls TEXT,
    trailer_urls TEXT,
    extrafanart_urls TEXT,
    scores TEXT,
    external_ids TEXT,
    source_urls TEXT,
    poster_resource_id INTEGER REFERENCES resource (id) ON DELETE SET NULL,
    cover_resource_id INTEGER REFERENCES resource (id) ON DELETE SET NULL,
    field_sources TEXT,
    genres TEXT,
    note TEXT,
    content_type TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- 旧单值塞进列表首项: 横版图归入 thumb_urls, 不是第二张竖版海报.
INSERT INTO metadata_rebuilt (
    id, number, title, plot, release_date, runtime_minutes, maker, label, series,
    poster_urls, thumb_urls, poster_resource_id, cover_resource_id,
    field_sources, genres, note, content_type, created_at, updated_at
)
SELECT
    id, number, title, plot, release_date, runtime_minutes, maker, label, series,
    CASE WHEN poster_url IS NULL THEN NULL ELSE json_array(poster_url) END,
    CASE WHEN cover_url IS NULL THEN NULL ELSE json_array(cover_url) END,
    poster_resource_id, cover_resource_id,
    field_sources, genres, note, content_type, created_at, updated_at
FROM metadata;

DROP TABLE metadata;

ALTER TABLE metadata_rebuilt RENAME TO metadata;

CREATE INDEX idx_metadata_updated_at ON metadata (updated_at DESC, id DESC);
CREATE INDEX idx_metadata_content_type ON metadata (content_type, updated_at DESC);
