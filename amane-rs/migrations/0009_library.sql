-- 媒体库登记: 一个根目录一条记录; 扫描结果按根目录整体替换.
CREATE TABLE library_root (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    path TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL
);

-- 扫描文件索引. path 是相对根目录的相对路径; kind 见 ScanKind.
-- rule/detail 记录无效文件命中的规则: rule 是规则名, detail 是命中的模式或说明.
CREATE TABLE library_file (
    root_id INTEGER NOT NULL REFERENCES library_root (id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    kind TEXT NOT NULL,
    rule TEXT,
    detail TEXT,
    PRIMARY KEY (root_id, path)
);

CREATE INDEX library_file_kind ON library_file (root_id, kind);
