use chrono::{DateTime, NaiveDate, Utc};
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
    pub plot: Option<String>,
    pub release_date: Option<NaiveDate>,
    pub runtime_minutes: Option<i32>,
    pub maker: Option<String>,
    pub label: Option<String>,
    pub series: Option<String>,
    pub poster_url: Option<String>,
    /// 海报存到本地后的资源标识; 为 `None` 时详情页只能引用远端地址.
    pub poster_resource_id: Option<i64>,
    /// 字段来源表, JSON 文本; 键是字段名, 值是来源标识.
    pub field_sources: Option<String>,
    pub content_type: ContentType,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 写入条目的字段集合.
///
/// 与 [`Metadata`] 分开: 写入时没有标识与时间戳。字段为 `None` 表示本次没有该值,
/// 是否覆盖既有值由各字段自己的规则决定。
#[derive(Debug, Clone, Default)]
pub struct MetadataInput<'a> {
    pub title: Option<&'a str>,
    pub plot: Option<&'a str>,
    pub release_date: Option<NaiveDate>,
    pub runtime_minutes: Option<i32>,
    pub maker: Option<&'a str>,
    pub label: Option<&'a str>,
    pub series: Option<&'a str>,
    pub poster_url: Option<&'a str>,
    /// `None` 表示本次不改动演员名单; `Some` 表示整体替换, 空列表即清空.
    pub actors: Option<&'a [String]>,
}

#[derive(Debug, Error)]
pub enum RepositoryError {
    #[error("数据库操作失败: {0}")]
    Database(#[from] sqlx::Error),
}

/// 条目仓储. 写操作经由写池, 读操作走读池.
#[derive(Debug, Clone)]
pub struct MetadataRepository {
    database: Database,
}

impl MetadataRepository {
    pub fn new(database: Database) -> Self {
        Self { database }
    }

    /// 按番号写入或更新. 空字段保留既有值, 演员名单整体替换.
    pub async fn upsert(
        &self,
        number: &MediaNumber,
        input: &MetadataInput<'_>,
    ) -> Result<Metadata, RepositoryError> {
        let now = Utc::now();
        let mut transaction = self.database.write().begin().await?;

        let row = sqlx::query_as::<_, Metadata>(
            r#"
            INSERT INTO metadata (
                number, title, plot, release_date, runtime_minutes,
                maker, label, series, poster_url, content_type, created_at, updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)
            ON CONFLICT(number) DO UPDATE SET
                title = COALESCE(excluded.title, metadata.title),
                plot = COALESCE(excluded.plot, metadata.plot),
                release_date = COALESCE(excluded.release_date, metadata.release_date),
                runtime_minutes = COALESCE(excluded.runtime_minutes, metadata.runtime_minutes),
                maker = COALESCE(excluded.maker, metadata.maker),
                label = COALESCE(excluded.label, metadata.label),
                series = COALESCE(excluded.series, metadata.series),
                poster_url = COALESCE(excluded.poster_url, metadata.poster_url),
                content_type = excluded.content_type,
                updated_at = excluded.updated_at
            RETURNING *
            "#,
        )
        .bind(number.as_str())
        .bind(input.title)
        .bind(input.plot)
        .bind(input.release_date)
        .bind(input.runtime_minutes)
        .bind(input.maker)
        .bind(input.label)
        .bind(input.series)
        .bind(input.poster_url)
        .bind(number.content_type())
        .bind(now)
        .fetch_one(&mut *transaction)
        .await?;

        if let Some(actors) = input.actors {
            sqlx::query("DELETE FROM metadata_actor WHERE metadata_id = ?1")
                .bind(row.id)
                .execute(&mut *transaction)
                .await?;

            for (index, name) in actors.iter().enumerate() {
                let actor_id = sqlx::query_scalar::<_, i64>(
                    r#"
                    INSERT INTO actor (name, created_at, updated_at)
                    VALUES (?1, ?2, ?2)
                    ON CONFLICT(name) DO UPDATE SET updated_at = excluded.updated_at
                    RETURNING id
                    "#,
                )
                .bind(name)
                .bind(now)
                .fetch_one(&mut *transaction)
                .await?;

                sqlx::query(
                    "INSERT INTO metadata_actor (metadata_id, actor_id, ordinal) VALUES (?1, ?2, ?3)",
                )
                .bind(row.id)
                .bind(actor_id)
                .bind(i64::try_from(index).unwrap_or(i64::MAX))
                .execute(&mut *transaction)
                .await?;
            }
        }

        transaction.commit().await?;
        Ok(row)
    }

    pub async fn find(&self, number: &MediaNumber) -> Result<Option<Metadata>, RepositoryError> {
        let row = sqlx::query_as::<_, Metadata>("SELECT * FROM metadata WHERE number = ?1")
            .bind(number.as_str())
            .fetch_optional(self.database.read())
            .await?;

        Ok(row)
    }

    /// 条目的演员名单, 按写入时的顺序.
    pub async fn actors(&self, metadata_id: i64) -> Result<Vec<String>, RepositoryError> {
        let names = sqlx::query_scalar::<_, String>(
            r#"
            SELECT actor.name FROM metadata_actor
            JOIN actor ON actor.id = metadata_actor.actor_id
            WHERE metadata_actor.metadata_id = ?1
            ORDER BY metadata_actor.ordinal ASC
            "#,
        )
        .bind(metadata_id)
        .fetch_all(self.database.read())
        .await?;

        Ok(names)
    }

    /// 记录字段来源.
    pub async fn set_field_sources(
        &self,
        metadata_id: i64,
        field_sources: &str,
    ) -> Result<(), RepositoryError> {
        sqlx::query("UPDATE metadata SET field_sources = ?1 WHERE id = ?2")
            .bind(field_sources)
            .bind(metadata_id)
            .execute(self.database.write())
            .await?;

        Ok(())
    }

    /// 用本次刮削的结果整体替换分站快照.
    pub async fn replace_snapshots(
        &self,
        metadata_id: i64,
        snapshots: &[(String, String)],
    ) -> Result<(), RepositoryError> {
        let mut transaction = self.database.write().begin().await?;
        let now = Utc::now();

        sqlx::query("DELETE FROM source_snapshot WHERE metadata_id = ?1")
            .bind(metadata_id)
            .execute(&mut *transaction)
            .await?;

        for (source_id, payload) in snapshots {
            sqlx::query(
                "INSERT INTO source_snapshot (metadata_id, source_id, payload, fetched_at) VALUES (?1, ?2, ?3, ?4)",
            )
            .bind(metadata_id)
            .bind(source_id)
            .bind(payload)
            .bind(now)
            .execute(&mut *transaction)
            .await?;
        }

        transaction.commit().await?;
        Ok(())
    }

    /// 条目的各来源快照, 按来源标识排序.
    pub async fn snapshots(
        &self,
        metadata_id: i64,
    ) -> Result<Vec<(String, String)>, RepositoryError> {
        let rows = sqlx::query_as::<_, (String, String)>(
            "SELECT source_id, payload FROM source_snapshot WHERE metadata_id = ?1 ORDER BY source_id ASC",
        )
        .bind(metadata_id)
        .fetch_all(self.database.read())
        .await?;

        Ok(rows)
    }

    /// 把条目指向一条资源. 资源表由资源仓储维护, 这里只写元数据侧的引用.
    pub async fn set_poster_resource(
        &self,
        metadata_id: i64,
        resource_id: i64,
    ) -> Result<(), RepositoryError> {
        sqlx::query("UPDATE metadata SET poster_resource_id = ?1 WHERE id = ?2")
            .bind(resource_id)
            .bind(metadata_id)
            .execute(self.database.write())
            .await?;

        Ok(())
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
    use super::{MetadataInput, MetadataRepository};
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

    fn input<'a>(title: Option<&'a str>, actors: &'a [String]) -> MetadataInput<'a> {
        MetadataInput {
            title,
            actors: Some(actors),
            ..MetadataInput::default()
        }
    }

    #[tokio::test]
    async fn upsert_is_idempotent_on_number() {
        let (_dir, repository) = repository().await;
        let number = MediaNumber::parse("ABC-123").expect("应解析成功");

        let first = repository
            .upsert(&number, &input(Some("初版标题"), &[]))
            .await
            .expect("首次写入应成功");
        let second = repository
            .upsert(&number, &input(None, &[]))
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
    async fn upsert_replaces_actor_list() {
        let (_dir, repository) = repository().await;
        let number = MediaNumber::parse("ABC-123").expect("应解析成功");
        let first = vec!["葵つかさ".to_owned(), "乙白さやか".to_owned()];
        let second = vec!["乙白さやか".to_owned()];

        let row = repository
            .upsert(&number, &input(Some("标题"), &first))
            .await
            .expect("应写入成功");
        repository
            .upsert(&number, &input(None, &second))
            .await
            .expect("应再次写入成功");

        assert_eq!(
            repository.actors(row.id).await.expect("应查询成功"),
            vec!["乙白さやか".to_owned()],
            "演员名单整体替换而不是累加"
        );
    }

    #[tokio::test]
    async fn absent_actor_list_keeps_existing() {
        let (_dir, repository) = repository().await;
        let number = MediaNumber::parse("ABC-123").expect("应解析成功");
        let actors = vec!["葵つかさ".to_owned()];
        let row = repository
            .upsert(&number, &input(Some("标题"), &actors))
            .await
            .expect("应写入成功");

        repository
            .upsert(
                &number,
                &MetadataInput {
                    title: Some("新标题"),
                    ..MetadataInput::default()
                },
            )
            .await
            .expect("应再次写入成功");

        assert_eq!(
            repository.actors(row.id).await.expect("应查询成功"),
            actors,
            "未提供演员名单时不应清空既有值"
        );
    }

    #[tokio::test]
    async fn finds_absent_number_as_none() {
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

    #[tokio::test]
    async fn list_is_ordered_by_update_time() {
        let (_dir, repository) = repository().await;
        for raw in ["ABC-001", "ABC-002", "ABC-003"] {
            let number = MediaNumber::parse(raw).expect("应解析成功");
            repository
                .upsert(&number, &input(None, &[]))
                .await
                .expect("写入应成功");
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }

        let rows = repository.list(10, 0).await.expect("应列出成功");
        let numbers: Vec<&str> = rows.iter().map(|row| row.number.as_str()).collect();

        assert_eq!(numbers, ["ABC-003", "ABC-002", "ABC-001"]);
    }
}
