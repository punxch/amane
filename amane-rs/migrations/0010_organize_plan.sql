-- 整理登记: 预览判「同名不同番号」的依据; 执行切片在移动成功后写入.
-- path 与 source_path 都是相对库根的 POSIX 路径; 同一库内同一路径只允许一个番号占用.
CREATE TABLE organize_plan (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    root_id INTEGER NOT NULL REFERENCES library_root (id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    number TEXT NOT NULL,
    source_path TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE (root_id, path)
);

CREATE INDEX organize_plan_number ON organize_plan (root_id, number);
