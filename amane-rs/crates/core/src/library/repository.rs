//! 媒体库登记与扫描索引的持久化.

use std::path::{Path, PathBuf};

use chrono::Utc;
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use super::model::{LibraryFile, LibraryRoot, ScanDiff, ScanKind};
use crate::db::Database;

/// 媒体库仓储错误.
#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    #[error("数据库操作失败: {0}")]
    Database(#[from] sqlx::Error),
    #[error("媒体库 {0} 不存在")]
    NotFound(i64),
    #[error("路径 {0} 已登记")]
    AlreadyRegistered(String),
}

/// 每个根目录的扫描结果统计.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KindCounts {
    pub media: i64,
    pub unwanted: i64,
    pub system: i64,
    pub unrecognized: i64,
    pub leftover_dir: i64,
}

impl KindCounts {
    /// 按类别顺序给出 (类别名, 数量), 界面据此渲染四类与未识别.
    pub fn as_pairs(&self) -> [(&'static str, i64); 5] {
        [
            ("media", self.media),
            ("unwanted", self.unwanted),
            ("system", self.system),
            ("unrecognized", self.unrecognized),
            ("leftover_dir", self.leftover_dir),
        ]
    }
}

/// 媒体库根目录与扫描索引的仓储.
#[derive(Debug, Clone)]
pub struct LibraryRepository {
    database: Database,
}

impl LibraryRepository {
    pub fn new(database: Database) -> Self {
        Self { database }
    }

    /// 登记一个根目录. 路径应先由调用方验证可读并规范化为绝对路径.
    pub async fn create(&self, path: &Path) -> Result<LibraryRoot, LibraryError> {
        let now = Utc::now();
        let row = sqlx::query_as::<_, LibraryRoot>(
            "INSERT INTO library_root (path, created_at) VALUES (?1, ?2) RETURNING *",
        )
        .bind(path.to_string_lossy().as_ref())
        .bind(now)
        .fetch_one(self.database.write())
        .await
        .map_err(|error| match &error {
            sqlx::Error::Database(database) if database.is_unique_violation() => {
                LibraryError::AlreadyRegistered(path.to_string_lossy().into_owned())
            }
            _ => LibraryError::Database(error),
        })?;

        Ok(row)
    }

    pub async fn list(&self) -> Result<Vec<LibraryRoot>, LibraryError> {
        let rows = sqlx::query_as::<_, LibraryRoot>("SELECT * FROM library_root ORDER BY id ASC")
            .fetch_all(self.database.read())
            .await?;

        Ok(rows)
    }

    /// 取媒体库登记; 不存在时报出可读原因.
    ///
    /// 处理函数与端点都需要这一句, 各写一遍会让文案漂移.
    pub async fn require(&self, id: i64) -> Result<LibraryRoot, LibraryError> {
        self.find(id)
            .await?
            .ok_or_else(|| LibraryError::NotFound(id))
    }

    pub async fn find(&self, id: i64) -> Result<Option<LibraryRoot>, LibraryError> {
        let row = sqlx::query_as::<_, LibraryRoot>("SELECT * FROM library_root WHERE id = ?1")
            .bind(id)
            .fetch_optional(self.database.read())
            .await?;

        Ok(row)
    }

    /// 移除登记. 扫描索引随外键级联删除; 磁盘文件不受影响.
    pub async fn delete(&self, id: i64) -> Result<(), LibraryError> {
        let result = sqlx::query("DELETE FROM library_root WHERE id = ?1")
            .bind(id)
            .execute(self.database.write())
            .await?;
        if result.rows_affected() == 0 {
            return Err(LibraryError::NotFound(id));
        }

        Ok(())
    }

    /// 从索引删除若干条目; 清理执行后调用, 让索引与磁盘保持一致.
    pub async fn delete_files(&self, root_id: i64, paths: &[String]) -> Result<u64, LibraryError> {
        let mut affected = 0u64;
        let mut transaction = self.database.write().begin().await?;
        for path in paths {
            let result = sqlx::query("DELETE FROM library_file WHERE root_id = ?1 AND path = ?2")
                .bind(root_id)
                .bind(path)
                .execute(&mut *transaction)
                .await?;
            affected += result.rows_affected();
        }
        transaction.commit().await?;

        Ok(affected)
    }

    /// 插入单条索引记录; 已存在同路径条目时保持原样.
    ///
    /// 恢复回收文件后调用: 原始归类已随清理删除, 先按未识别登记, 重新扫描再归类.
    pub async fn insert_file(&self, root_id: i64, file: &LibraryFile) -> Result<(), LibraryError> {
        sqlx::query(
            "INSERT OR IGNORE INTO library_file (root_id, path, kind, rule, detail) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(root_id)
        .bind(&file.path)
        .bind(file.kind)
        .bind(&file.rule)
        .bind(&file.detail)
        .execute(self.database.write())
        .await?;

        Ok(())
    }

    /// 以本次扫描结果整体替换一个根目录的索引, 并给出相对上次的增量.
    ///
    /// 比较按路径进行: 归类规则变了, 已存在的路径也算不上新增.
    /// 删除与写入在同一事务内, 中途失败不会留下半个库的索引.
    pub async fn replace_files(
        &self,
        root_id: i64,
        files: &[LibraryFile],
        leftover_dirs: &[PathBuf],
    ) -> Result<ScanDiff, LibraryError> {
        if self.find(root_id).await?.is_none() {
            return Err(LibraryError::NotFound(root_id));
        }

        let mut entries: Vec<LibraryFile> = files.to_vec();
        entries.extend(
            leftover_dirs
                .iter()
                .map(|path| LibraryFile::new(path.to_string_lossy(), ScanKind::LeftoverDir)),
        );

        let mut transaction = self.database.write().begin().await?;
        let previous: std::collections::BTreeSet<String> =
            sqlx::query("SELECT path FROM library_file WHERE root_id = ?1")
                .bind(root_id)
                .fetch_all(&mut *transaction)
                .await?
                .into_iter()
                .filter_map(|row: SqliteRow| row.try_get::<String, _>("path").ok())
                .collect();

        let current: std::collections::BTreeSet<String> =
            entries.iter().map(|file| file.path.clone()).collect();
        let diff = ScanDiff {
            added: current.difference(&previous).cloned().collect(),
            removed: previous.difference(&current).cloned().collect(),
        };

        sqlx::query("DELETE FROM library_file WHERE root_id = ?1")
            .bind(root_id)
            .execute(&mut *transaction)
            .await?;
        for file in &entries {
            sqlx::query(
                "INSERT INTO library_file (root_id, path, kind, rule, detail) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .bind(root_id)
            .bind(&file.path)
            .bind(file.kind)
            .bind(&file.rule)
            .bind(&file.detail)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;

        Ok(diff)
    }

    /// 分页列出索引. `kind` 为空时列出全部.
    pub async fn files(
        &self,
        root_id: i64,
        kind: Option<ScanKind>,
        limit: u32,
        offset: u32,
    ) -> Result<(i64, Vec<LibraryFile>), LibraryError> {
        let total: i64 = match kind {
            Some(kind) => {
                sqlx::query_scalar(
                    "SELECT COUNT(*) FROM library_file WHERE root_id = ?1 AND kind = ?2",
                )
                .bind(root_id)
                .bind(kind)
                .fetch_one(self.database.read())
                .await?
            }
            None => {
                sqlx::query_scalar("SELECT COUNT(*) FROM library_file WHERE root_id = ?1")
                    .bind(root_id)
                    .fetch_one(self.database.read())
                    .await?
            }
        };

        let rows: Vec<LibraryFile> = match kind {
            Some(kind) => {
                sqlx::query_as(
                    "SELECT path, kind, rule, detail FROM library_file \
                     WHERE root_id = ?1 AND kind = ?2 \
                     ORDER BY path ASC LIMIT ?3 OFFSET ?4",
                )
                .bind(root_id)
                .bind(kind)
                .bind(limit)
                .bind(offset)
                .fetch_all(self.database.read())
                .await?
            }
            None => {
                sqlx::query_as(
                    "SELECT path, kind, rule, detail FROM library_file \
                     WHERE root_id = ?1 ORDER BY path ASC LIMIT ?2 OFFSET ?3",
                )
                .bind(root_id)
                .bind(limit)
                .bind(offset)
                .fetch_all(self.database.read())
                .await?
            }
        };

        Ok((total, rows))
    }

    /// 索引里的全部文件, 不设上限.
    ///
    /// 整理预览与清理清单需要完整集合: 按页取会让超出页大小的文件既不整理也不清理,
    /// 而界面上看不出被截断.
    pub async fn all_files(
        &self,
        root_id: i64,
        kind: Option<ScanKind>,
    ) -> Result<Vec<LibraryFile>, LibraryError> {
        let rows: Vec<LibraryFile> = match kind {
            Some(kind) => {
                sqlx::query_as(
                    "SELECT path, kind, rule, detail FROM library_file \
                     WHERE root_id = ?1 AND kind = ?2 ORDER BY path ASC",
                )
                .bind(root_id)
                .bind(kind)
                .fetch_all(self.database.read())
                .await?
            }
            None => {
                sqlx::query_as(
                    "SELECT path, kind, rule, detail FROM library_file \
                     WHERE root_id = ?1 ORDER BY path ASC",
                )
                .bind(root_id)
                .fetch_all(self.database.read())
                .await?
            }
        };

        Ok(rows)
    }

    /// 每个类别的条数, 登记列表页据此显示四类统计.
    pub async fn counts(&self, root_id: i64) -> Result<KindCounts, LibraryError> {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT kind, COUNT(*) FROM library_file WHERE root_id = ?1 GROUP BY kind",
        )
        .bind(root_id)
        .fetch_all(self.database.read())
        .await?;

        let mut counts = KindCounts::default();
        for (kind, count) in rows {
            match ScanKind::parse(&kind) {
                Some(ScanKind::Media) => counts.media = count,
                Some(ScanKind::Unwanted) => counts.unwanted = count,
                Some(ScanKind::System) => counts.system = count,
                Some(ScanKind::Unrecognized) => counts.unrecognized = count,
                Some(ScanKind::LeftoverDir) => counts.leftover_dir = count,
                None => continue,
            }
        }

        Ok(counts)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::super::model::{LibraryFile, ScanKind, UnwantedRule};
    use super::{KindCounts, LibraryRepository};
    use crate::db::Database;

    async fn repository() -> (tempfile::TempDir, LibraryRepository) {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let database = Database::open(&dir.path().join("amane.db"))
            .await
            .expect("应打开数据库");
        (dir, LibraryRepository::new(database))
    }

    #[tokio::test]
    async fn duplicate_path_is_rejected_with_reason() {
        let (_dir, repository) = repository().await;
        let path = std::path::Path::new("/tmp/amane-library");
        repository.create(path).await.expect("首次登记应成功");

        let error = repository.create(path).await.expect_err("重复登记应被拒绝");
        assert!(
            error.to_string().contains("已登记"),
            "错误应说明原因, 实际 {error}"
        );
    }

    #[tokio::test]
    async fn replace_files_reports_added_and_removed() {
        let (_dir, repository) = repository().await;
        let root = repository
            .create(std::path::Path::new("/tmp/amane-library"))
            .await
            .expect("应登记成功");

        let first = repository
            .replace_files(
                root.id,
                &[
                    LibraryFile::new("a.mp4", ScanKind::Media),
                    LibraryFile::new("b.txt", ScanKind::Unrecognized),
                ],
                &[PathBuf::from("empty")],
            )
            .await
            .expect("应写入成功");
        assert_eq!(
            first.added,
            vec!["a.mp4".to_owned(), "b.txt".to_owned(), "empty".to_owned()],
            "首次扫描相对空索引全部记为新增"
        );
        assert!(first.removed.is_empty());

        let second = repository
            .replace_files(root.id, &[LibraryFile::new("a.mp4", ScanKind::Media)], &[])
            .await
            .expect("应写入成功");
        assert_eq!(second.added, Vec::<String>::new());
        assert_eq!(second.removed, vec!["b.txt".to_owned(), "empty".to_owned()]);
    }

    #[tokio::test]
    async fn all_files_ignores_page_size() {
        let (_dir, repository) = repository().await;
        let root = repository
            .create(std::path::Path::new("/tmp/amane-library"))
            .await
            .expect("应登记成功");
        let files: Vec<LibraryFile> = (0..260)
            .map(|index| LibraryFile::new(format!("{index:03}.mp4"), ScanKind::Media))
            .collect();
        repository
            .replace_files(root.id, &files, &[])
            .await
            .expect("应写入成功");

        let (total, page) = repository
            .files(root.id, Some(ScanKind::Media), 200, 0)
            .await
            .expect("应查询成功");
        let all = repository
            .all_files(root.id, Some(ScanKind::Media))
            .await
            .expect("应查询成功");

        assert_eq!(total, 260);
        assert_eq!(page.len(), 200, "分页查询受上限约束");
        assert_eq!(all.len(), 260, "全量查询不受分页上限约束");
    }

    #[tokio::test]
    async fn counts_and_files_cover_all_kinds() {
        let (_dir, repository) = repository().await;
        let root = repository
            .create(std::path::Path::new("/tmp/amane-library"))
            .await
            .expect("应登记成功");
        repository
            .replace_files(
                root.id,
                &[
                    LibraryFile::new("a.mp4", ScanKind::Media),
                    LibraryFile::unwanted("ads.mp4", UnwantedRule::Blacklist, "(?i)ads".to_owned()),
                    LibraryFile::new(".DS_Store", ScanKind::System),
                    LibraryFile::new("note.txt", ScanKind::Unrecognized),
                ],
                &[PathBuf::from("empty")],
            )
            .await
            .expect("应写入成功");

        let counts = repository.counts(root.id).await.expect("应统计成功");
        assert_eq!(
            counts,
            KindCounts {
                media: 1,
                unwanted: 1,
                system: 1,
                unrecognized: 1,
                leftover_dir: 1,
            }
        );

        let (total, rows) = repository
            .files(root.id, Some(ScanKind::Unwanted), 10, 0)
            .await
            .expect("应查询成功");
        assert_eq!(total, 1);
        assert_eq!(rows[0].rule.as_deref(), Some("blacklist"));
        assert_eq!(rows[0].detail.as_deref(), Some("(?i)ads"));
    }

    #[tokio::test]
    async fn delete_cascades_index_and_reports_missing() {
        let (_dir, repository) = repository().await;
        let root = repository
            .create(std::path::Path::new("/tmp/amane-library"))
            .await
            .expect("应登记成功");
        repository
            .replace_files(root.id, &[LibraryFile::new("a.mp4", ScanKind::Media)], &[])
            .await
            .expect("应写入成功");

        repository.delete(root.id).await.expect("应删除成功");
        let error = repository
            .replace_files(root.id, &[], &[])
            .await
            .expect_err("已删除的库不应再接受写入");
        assert!(matches!(error, super::LibraryError::NotFound(_)));
        let error = repository.delete(root.id).await.expect_err("应报不存在");
        assert!(error.to_string().contains("不存在"));
    }
}
