use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use thiserror::Error;

use crate::content_type::ContentType;
use crate::db::Database;
use crate::number::MediaNumber;

/// 番号条目. 以番号为键, 承载多源聚合后的影片信息.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Metadata {
    pub id: i64,
    pub number: String,
    pub title: Option<String>,
    pub content_type: ContentType,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Error)]
pub enum RepositoryError {
    #[error("数据库操作失败: {0}")]
    Database(#[from] sqlx::Error),
}

/// 条目仓储. 写操作走写池, 读操作走读池.
#[derive(Debug, Clone)]
pub struct MetadataRepository {
    database: Database,
}

impl MetadataRepository {
    pub fn new(database: Database) -> Self {
        Self { database }
    }

    /// 按番号写入或更新. `title` 为空时保留既有值.
    pub async fn upsert(
        &self,
        number: &MediaNumber,
        title: Option<&str>,
    ) -> Result<Metadata, RepositoryError> {
        let now = Utc::now();
        let row = sqlx::query_as::<_, Metadata>(
            r#"
            INSERT INTO metadata (number, title, content_type, created_at, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?4)
            ON CONFLICT(number) DO UPDATE SET
                title = COALESCE(excluded.title, metadata.title),
                content_type = excluded.content_type,
                updated_at = excluded.updated_at
            RETURNING *
            "#,
        )
        .bind(number.as_str())
        .bind(title)
        .bind(number.content_type())
        .bind(now)
        .fetch_one(self.database.write())
        .await?;

        Ok(row)
    }

    pub async fn find(&self, number: &MediaNumber) -> Result<Option<Metadata>, RepositoryError> {
        let row = sqlx::query_as::<_, Metadata>("SELECT * FROM metadata WHERE number = ?1")
            .bind(number.as_str())
            .fetch_optional(self.database.read())
            .await?;

        Ok(row)
    }

    pub async fn list(&self, limit: u32, offset: u32) -> Result<Vec<Metadata>, RepositoryError> {
        let rows = sqlx::query_as::<_, Metadata>(
            "SELECT * FROM metadata ORDER BY updated_at DESC, id DESC LIMIT ?1 OFFSET ?2",
        )
        .bind(i64::from(limit))
        .bind(i64::from(offset))
        .fetch_all(self.database.read())
        .await?;

        Ok(rows)
    }

    pub async fn count(&self) -> Result<i64, RepositoryError> {
        let total = sqlx::query_scalar("SELECT count(*) FROM metadata")
            .fetch_one(self.database.read())
            .await?;

        Ok(total)
    }
}

#[cfg(test)]
mod tests {
    use super::MetadataRepository;
    use crate::content_type::ContentType;
    use crate::db::Database;
    use crate::number::MediaNumber;

    async fn repository() -> (tempfile::TempDir, MetadataRepository) {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let database = Database::open(&dir.path().join("amane.db"))
            .await
            .expect("应打开数据库");
        (dir, MetadataRepository::new(database))
    }

    #[tokio::test]
    async fn upsert_is_idempotent_on_number() {
        let (_dir, repository) = repository().await;
        let number = MediaNumber::parse("ABC-123").expect("应解析成功");

        let first = repository
            .upsert(&number, Some("初版标题"))
            .await
            .expect("首次写入应成功");
        let second = repository
            .upsert(&number, None)
            .await
            .expect("重复写入应更新而非插入");

        assert_eq!(first.id, second.id, "同一番号必须复用同一条记录");
        assert_eq!(
            second.title.as_deref(),
            Some("初版标题"),
            "空标题不应覆盖既有值"
        );
        assert_eq!(second.content_type, ContentType::Censored);
        assert_eq!(repository.count().await.expect("应统计成功"), 1);
    }

    #[tokio::test]
    async fn list_is_ordered_by_update_time() {
        let (_dir, repository) = repository().await;
        for raw in ["ABC-001", "ABC-002", "ABC-003"] {
            let number = MediaNumber::parse(raw).expect("应解析成功");
            repository.upsert(&number, None).await.expect("写入应成功");
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }

        let rows = repository.list(10, 0).await.expect("应列出成功");
        let numbers: Vec<&str> = rows.iter().map(|row| row.number.as_str()).collect();

        assert_eq!(numbers, ["ABC-003", "ABC-002", "ABC-001"]);
    }

    #[tokio::test]
    async fn find_returns_none_for_absent_number() {
        let (_dir, repository) = repository().await;
        let number = MediaNumber::parse("ABC-999").expect("应解析成功");

        assert!(
            repository
                .find(&number)
                .await
                .expect("查询应成功")
                .is_none()
        );
    }
}
