use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, QueryBuilder, Sqlite};
use thiserror::Error;

use crate::content_type::ContentType;
use crate::db::Database;
use crate::number::MediaNumber;

/// 列表筛选条件. 各项为空表示不限制, 组合时取交集.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MetadataFilter {
    pub content_type: Option<ContentType>,
    /// 贡献过该番号条目的来源标识.
    pub source: Option<String>,
    /// 关键字: 匹配番号或标题, 忽略大小写.
    pub keyword: Option<String>,
}

impl MetadataFilter {
    /// 关键字转成 LIKE 模式. `%` 与 `_` 是通配符, 出现在关键字里必须转义,
    /// 否则用户搜 `100%` 会命中所有番号条目.
    fn pattern(&self) -> Option<String> {
        let keyword = self.keyword.as_deref()?.trim();
        if keyword.is_empty() {
            return None;
        }

        let mut escaped = String::with_capacity(keyword.len() + 2);
        escaped.push('%');
        for character in keyword.chars() {
            if matches!(character, '\\' | '%' | '_') {
                escaped.push('\\');
            }
            escaped.push(character);
        }
        escaped.push('%');

        Some(escaped)
    }
}

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

/// 写入番号条目的字段集合.
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

/// 把筛选条件追加到 `WHERE` 子句; `list` 与 `count` 共用, 避免两处条件漂移.
fn push_conditions(builder: &mut QueryBuilder<'_, Sqlite>, filter: &MetadataFilter) {
    builder.push(" WHERE 1 = 1");

    if let Some(content_type) = filter.content_type {
        builder.push(" AND content_type = ").push_bind(content_type);
    }

    if let Some(source) = filter.source.as_deref() {
        builder
            .push(" AND EXISTS (SELECT 1 FROM source_snapshot WHERE source_snapshot.metadata_id = metadata.id AND source_snapshot.source_id = ")
            .push_bind(source.to_owned())
            .push(")");
    }

    if let Some(pattern) = filter.pattern() {
        builder
            .push(" AND (number LIKE ")
            .push_bind(pattern.clone())
            .push(" ESCAPE '\\' OR title LIKE ")
            .push_bind(pattern)
            .push(" ESCAPE '\\')");
    }
}

#[derive(Debug, Error)]
pub enum RepositoryError {
    #[error("数据库操作失败: {0}")]
    Database(#[from] sqlx::Error),
}

/// 番号条目仓储. 写操作经由写池, 读操作走读池.
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

    /// 番号条目的演员名单, 按写入时的顺序.
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

    /// 番号条目的各来源快照, 按来源标识排序.
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

    /// 把番号条目指向一条资源. 资源表由资源仓储维护, 这里只写元数据侧的引用.
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

    pub async fn list(
        &self,
        filter: &MetadataFilter,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<Metadata>, RepositoryError> {
        let mut builder = QueryBuilder::<Sqlite>::new("SELECT * FROM metadata");
        push_conditions(&mut builder, filter);
        builder
            .push(" ORDER BY updated_at DESC, id DESC LIMIT ")
            .push_bind(i64::from(limit))
            .push(" OFFSET ")
            .push_bind(i64::from(offset));

        let rows = builder
            .build_query_as::<Metadata>()
            .fetch_all(self.database.read())
            .await?;

        Ok(rows)
    }

    /// 命中总数. 与 `list` 使用同一套条件, 分页时两者必须一致.
    pub async fn count(&self, filter: &MetadataFilter) -> Result<i64, RepositoryError> {
        let mut builder = QueryBuilder::<Sqlite>::new("SELECT count(*) FROM metadata");
        push_conditions(&mut builder, filter);

        let total = builder
            .build_query_scalar::<i64>()
            .fetch_one(self.database.read())
            .await?;

        Ok(total)
    }
}

#[cfg(test)]
mod tests {
    use super::{MetadataFilter, MetadataInput, MetadataRepository};
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
        assert_eq!(
            repository
                .count(&MetadataFilter::default())
                .await
                .expect("应统计成功"),
            1
        );
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

    /// 创建三条不同类型、带来源快照的番号条目, 供筛选用例共用.
    async fn seeded() -> (tempfile::TempDir, MetadataRepository) {
        let (dir, repository) = repository().await;

        // 类型由番号自身决定 (见 `MediaNumber::content_type`), 因此这里按形态选择番号.
        let cases = [
            ("ABC-001", "第一部", &["dmm", "javdb"][..]),
            ("FC2-PPV-1234567", "第二部", &["javdb"][..]),
            ("XYZ-100", "百分比 100% 的片子", &[][..]),
        ];
        for (raw, title, sources) in cases {
            let number = MediaNumber::parse(raw).expect("应解析成功");
            let input = MetadataInput {
                title: Some(title),
                ..MetadataInput::default()
            };
            let row = repository
                .upsert(&number, &input)
                .await
                .expect("应写入成功");
            let snapshots: Vec<(String, String)> = sources
                .iter()
                .map(|source| ((*source).to_owned(), "{}".to_owned()))
                .collect();
            repository
                .replace_snapshots(row.id, &snapshots)
                .await
                .expect("应写入快照");
        }

        (dir, repository)
    }

    #[tokio::test]
    async fn list_matches_each_filter() {
        let (_dir, repository) = seeded().await;
        let cases: &[(&str, MetadataFilter, &[&str])] = &[
            (
                "按类型",
                MetadataFilter {
                    content_type: Some(ContentType::Fc2),
                    ..MetadataFilter::default()
                },
                &["FC2-1234567"],
            ),
            (
                "按来源",
                MetadataFilter {
                    source: Some("dmm".to_owned()),
                    ..MetadataFilter::default()
                },
                &["ABC-001"],
            ),
            (
                "番号关键字",
                MetadataFilter {
                    keyword: Some("abc-00".to_owned()),
                    ..MetadataFilter::default()
                },
                &["ABC-001"],
            ),
            (
                "标题关键字",
                MetadataFilter {
                    keyword: Some("第二部".to_owned()),
                    ..MetadataFilter::default()
                },
                &["FC2-1234567"],
            ),
            (
                "关键字忽略大小写",
                MetadataFilter {
                    keyword: Some("fc2".to_owned()),
                    ..MetadataFilter::default()
                },
                &["FC2-1234567"],
            ),
            (
                "通配符按字面",
                MetadataFilter {
                    keyword: Some("100%".to_owned()),
                    ..MetadataFilter::default()
                },
                &["XYZ-100"],
            ),
            (
                "空白关键字不过滤",
                MetadataFilter {
                    keyword: Some("   ".to_owned()),
                    ..MetadataFilter::default()
                },
                &["ABC-001", "FC2-1234567", "XYZ-100"],
            ),
            (
                "三个条件取交集",
                MetadataFilter {
                    content_type: Some(ContentType::Censored),
                    source: Some("javdb".to_owned()),
                    keyword: Some("ABC".to_owned()),
                },
                &["ABC-001"],
            ),
            (
                "没有命中",
                MetadataFilter {
                    keyword: Some("不存在的词".to_owned()),
                    ..MetadataFilter::default()
                },
                &[],
            ),
        ];

        for (name, filter, expected) in cases {
            let rows = repository.list(filter, 10, 0).await.expect("应列出成功");
            let mut actual: Vec<&str> = rows.iter().map(|row| row.number.as_str()).collect();
            let mut expected = expected.to_vec();
            actual.sort_unstable();
            expected.sort_unstable();

            assert_eq!(actual, expected, "用例: {name}");
            let total = repository.count(filter).await.expect("应统计成功");
            assert_eq!(
                usize::try_from(total).unwrap_or_default(),
                expected.len(),
                "总数应与列表同源: {name}"
            );
        }
    }

    #[tokio::test]
    async fn offset_past_end_yields_no_rows() {
        let (_dir, repository) = seeded().await;

        let rows = repository
            .list(&MetadataFilter::default(), 10, 99)
            .await
            .expect("应列出成功");
        let total = repository
            .count(&MetadataFilter::default())
            .await
            .expect("应统计成功");

        assert!(rows.is_empty(), "越界偏移应返回空页");
        assert_eq!(total, 3, "总数不受偏移影响");
    }

    #[tokio::test]
    async fn filters_combine_as_intersection() {
        let (_dir, repository) = seeded().await;

        let rows = repository
            .list(
                &MetadataFilter {
                    content_type: Some(ContentType::Censored),
                    source: Some("javdb".to_owned()),
                    keyword: Some("ABC".to_owned()),
                },
                10,
                0,
            )
            .await
            .expect("应列出成功");

        assert_eq!(rows.len(), 1, "三个条件应同时生效");
        assert_eq!(rows[0].number, "ABC-001");
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

        let rows = repository
            .list(&MetadataFilter::default(), 10, 0)
            .await
            .expect("应列出成功");
        let numbers: Vec<&str> = rows.iter().map(|row| row.number.as_str()).collect();

        assert_eq!(numbers, ["ABC-003", "ABC-002", "ABC-001"]);
    }
}
