//! 资源: 从来源下载到本地的图片, 以远端地址为键去重.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use thiserror::Error;

use crate::db::Database;

/// 一条已写入磁盘的资源.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Resource {
    pub id: i64,
    /// 远端地址, 同时是去重键.
    pub url: String,
    /// 资源目录下的文件名.
    pub file_name: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Error)]
pub enum ResourceError {
    #[error("数据库操作失败: {0}")]
    Database(#[from] sqlx::Error),
}

/// 资源仓储. 只记录已写入磁盘的文件, 下载本身不在这里.
#[derive(Debug, Clone)]
pub struct ResourceRepository {
    database: Database,
}

impl ResourceRepository {
    pub fn new(database: Database) -> Self {
        Self { database }
    }

    pub async fn find_by_url(&self, url: &str) -> Result<Option<Resource>, ResourceError> {
        let row = sqlx::query_as::<_, Resource>("SELECT * FROM resource WHERE url = ?1")
            .bind(url)
            .fetch_optional(self.database.read())
            .await?;

        Ok(row)
    }

    pub async fn find(&self, id: i64) -> Result<Option<Resource>, ResourceError> {
        let row = sqlx::query_as::<_, Resource>("SELECT * FROM resource WHERE id = ?1")
            .bind(id)
            .fetch_optional(self.database.read())
            .await?;

        Ok(row)
    }

    /// 登记一条资源. 文件名在取得标识后才确定, 因此先插入再回写.
    pub async fn reserve(
        &self,
        url: &str,
        content_type: &str,
        size_bytes: i64,
    ) -> Result<Resource, ResourceError> {
        let row = sqlx::query_as::<_, Resource>(
            r#"
            INSERT INTO resource (url, file_name, content_type, size_bytes, created_at)
            VALUES (?1, '', ?2, ?3, ?4)
            ON CONFLICT(url) DO UPDATE SET
                content_type = excluded.content_type,
                size_bytes = excluded.size_bytes
            RETURNING *
            "#,
        )
        .bind(url)
        .bind(content_type)
        .bind(size_bytes)
        .bind(Utc::now())
        .fetch_one(self.database.write())
        .await?;

        Ok(row)
    }

    pub async fn set_file_name(&self, id: i64, file_name: &str) -> Result<(), ResourceError> {
        sqlx::query("UPDATE resource SET file_name = ?1 WHERE id = ?2")
            .bind(file_name)
            .bind(id)
            .execute(self.database.write())
            .await?;

        Ok(())
    }

    pub async fn delete(&self, id: i64) -> Result<(), ResourceError> {
        sqlx::query("DELETE FROM resource WHERE id = ?1")
            .bind(id)
            .execute(self.database.write())
            .await?;

        Ok(())
    }

    /// 把条目指向某条资源; `None` 表示解除引用.
    pub async fn link_poster(
        &self,
        metadata_id: i64,
        resource_id: Option<i64>,
    ) -> Result<(), ResourceError> {
        sqlx::query("UPDATE metadata SET poster_resource_id = ?1 WHERE id = ?2")
            .bind(resource_id)
            .bind(metadata_id)
            .execute(self.database.write())
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::ResourceRepository;
    use crate::db::Database;

    async fn repository() -> (tempfile::TempDir, ResourceRepository) {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let database = Database::open(&dir.path().join("amane.db"))
            .await
            .expect("应打开数据库");
        (dir, ResourceRepository::new(database))
    }

    #[tokio::test]
    async fn reserve_is_idempotent_on_url() {
        let (_dir, repository) = repository().await;

        let first = repository
            .reserve("https://example.invalid/a.jpg", "image/jpeg", 10)
            .await
            .expect("应登记成功");
        let second = repository
            .reserve("https://example.invalid/a.jpg", "image/jpeg", 10)
            .await
            .expect("重复登记应复用");

        assert_eq!(first.id, second.id, "同一地址必须复用同一条资源");
    }

    #[tokio::test]
    async fn file_name_is_written_after_reserve() {
        let (_dir, repository) = repository().await;
        let resource = repository
            .reserve("https://example.invalid/b.png", "image/png", 20)
            .await
            .expect("应登记成功");

        repository
            .set_file_name(resource.id, "1.png")
            .await
            .expect("应写入成功");
        let stored = repository
            .find(resource.id)
            .await
            .expect("应查询成功")
            .expect("应存在");

        assert_eq!(stored.file_name, "1.png");
    }

    #[tokio::test]
    async fn re_reserve_refreshes_type_and_size() {
        let (_dir, repository) = repository().await;
        let url = "https://example.invalid/c.jpg";

        repository
            .reserve(url, "text/html", 4)
            .await
            .expect("应登记成功");
        let refreshed = repository
            .reserve(url, "image/jpeg", 900)
            .await
            .expect("应再次登记成功");

        assert_eq!(refreshed.content_type, "image/jpeg", "重复登记应刷新类型");
        assert_eq!(refreshed.size_bytes, 900, "重复登记应刷新大小");
    }

    #[tokio::test]
    async fn finds_absent_url_as_none() {
        let (_dir, repository) = repository().await;

        assert!(
            repository
                .find_by_url("https://example.invalid/none.jpg")
                .await
                .expect("应查询成功")
                .is_none()
        );
    }
}
