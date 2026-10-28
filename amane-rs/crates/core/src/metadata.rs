use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, QueryBuilder, Sqlite, SqliteConnection};
use thiserror::Error;

use crate::content_type::ContentType;
use crate::db::Database;
use crate::lockable::LockableField;
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
    /// 用户备注, 只有人工写入.
    pub note: Option<String>,
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

impl MetadataInput<'_> {
    /// 本次输入是否带有该字段的值. 用于区分「因锁定而保留」与「本次本来就没提供」.
    pub fn has_value(&self, field: LockableField) -> bool {
        match field {
            LockableField::Title => self.title.is_some(),
            LockableField::Plot => self.plot.is_some(),
            LockableField::ReleaseDate => self.release_date.is_some(),
            LockableField::RuntimeMinutes => self.runtime_minutes.is_some(),
            LockableField::Maker => self.maker.is_some(),
            LockableField::Label => self.label.is_some(),
            LockableField::Series => self.series.is_some(),
            LockableField::PosterUrl => self.poster_url.is_some(),
        }
    }
}

impl<'a> MetadataInput<'a> {
    /// 去掉被锁定字段后的输入. 锁定的字段以 `None` 参与写入, 由 `COALESCE` 保留原值.
    pub fn without_locked(&self, locks: &BTreeSet<LockableField>) -> Self {
        fn keep<'value>(
            locks: &BTreeSet<LockableField>,
            field: LockableField,
            value: Option<&'value str>,
        ) -> Option<&'value str> {
            if locks.contains(&field) { None } else { value }
        }

        Self {
            title: keep(locks, LockableField::Title, self.title),
            plot: keep(locks, LockableField::Plot, self.plot),
            release_date: if locks.contains(&LockableField::ReleaseDate) {
                None
            } else {
                self.release_date
            },
            runtime_minutes: if locks.contains(&LockableField::RuntimeMinutes) {
                None
            } else {
                self.runtime_minutes
            },
            maker: keep(locks, LockableField::Maker, self.maker),
            label: keep(locks, LockableField::Label, self.label),
            series: keep(locks, LockableField::Series, self.series),
            poster_url: keep(locks, LockableField::PosterUrl, self.poster_url),
            actors: self.actors,
        }
    }
}

/// 人工修订写入 `field_sources` 的取值. 详情页据此区分「自动刮削」与「人工修订」.
pub const MANUAL_SOURCE: &str = "manual";

fn blank_to_none(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

/// 一次刮削要写入的内容: 字段值、字段来源, 以及是否被锁定约束.
pub struct ScrapeWrite<'a> {
    pub number: &'a MediaNumber,
    pub input: MetadataInput<'a>,
    /// 字段名 → 来源标识. 锁定字段的条目会被忽略.
    pub field_sources: BTreeMap<String, String>,
}

/// 刮削写入的结果.
pub struct ScrapeApplied {
    pub row: Metadata,
    /// 因锁定而未改动的字段, 供任务日志说明原因.
    pub kept: Vec<LockableField>,
}

/// 手工修订. `Some` 表示修改, 空字符串表示清空; `None` 表示不改动.
#[derive(Debug, Default)]
pub struct MetadataPatch<'a> {
    pub title: Option<&'a str>,
    pub plot: Option<&'a str>,
    pub release_date: Option<&'a str>,
    pub runtime_minutes: Option<i32>,
    pub maker: Option<&'a str>,
    pub label: Option<&'a str>,
    pub series: Option<&'a str>,
    pub note: Option<&'a str>,
}

/// 合并字段来源: 以既有表为底, 跳过被锁定字段后写入本次来源.
fn merge_sources(
    existing: Option<&str>,
    updates: &BTreeMap<String, String>,
    locked: &BTreeSet<LockableField>,
) -> Result<String, RepositoryError> {
    let mut merged: BTreeMap<String, String> = existing
        .and_then(|raw| serde_json::from_str(raw).ok())
        .unwrap_or_default();

    for (field, source) in updates {
        let blocked = LockableField::parse(field).is_some_and(|field| locked.contains(&field));
        if !blocked {
            merged.insert(field.clone(), source.clone());
        }
    }

    serde_json::to_string(&merged).map_err(|error| RepositoryError::InvalidInput(error.to_string()))
}

/// 写入或更新一行, 调用方负责事务边界.
async fn upsert_row(
    transaction: &mut sqlx::Transaction<'_, Sqlite>,
    number: &MediaNumber,
    input: &MetadataInput<'_>,
) -> Result<Metadata, sqlx::Error> {
    let now = Utc::now();
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
    .fetch_one(&mut **transaction)
    .await?;

    if let Some(actors) = input.actors {
        sqlx::query("DELETE FROM metadata_actor WHERE metadata_id = ?1")
            .bind(row.id)
            .execute(&mut **transaction)
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
            .fetch_one(&mut **transaction)
            .await?;

            sqlx::query(
                "INSERT INTO metadata_actor (metadata_id, actor_id, ordinal) VALUES (?1, ?2, ?3)",
            )
            .bind(row.id)
            .bind(actor_id)
            .bind(i64::try_from(index).unwrap_or(i64::MAX))
            .execute(&mut **transaction)
            .await?;
        }
    }

    Ok(row)
}

/// 读取某个番号条目的锁定字段. 事务与连接池共用同一份实现.
async fn read_locks(
    connection: &mut SqliteConnection,
    metadata_id: i64,
) -> Result<BTreeSet<LockableField>, sqlx::Error> {
    let fields: Vec<String> =
        sqlx::query_scalar("SELECT field FROM metadata_lock WHERE metadata_id = ?1")
            .bind(metadata_id)
            .fetch_all(&mut *connection)
            .await?;

    Ok(fields
        .into_iter()
        .filter_map(|field| LockableField::parse(&field))
        .collect())
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
    #[error("番号条目不存在")]
    NotFound,
    #[error("字段取值不合法: {0}")]
    InvalidInput(String),
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
        let mut transaction = self.database.write().begin().await?;
        let row = upsert_row(&mut transaction, number, input).await?;
        transaction.commit().await?;

        Ok(row)
    }

    /// 写回一次刮削的结果, 跳过被锁定的字段并合并字段来源.
    ///
    /// 锁的读取、写入与字段来源合并必须在同一事务里: 分成两次读会让「加锁」与「刮削」
    /// 交错时丢掉刚写入的人工值.
    pub async fn apply_scraped(
        &self,
        write: &ScrapeWrite<'_>,
    ) -> Result<ScrapeApplied, RepositoryError> {
        let mut transaction = self.database.write().begin().await?;

        let existing_id: Option<i64> =
            sqlx::query_scalar("SELECT id FROM metadata WHERE number = ?1")
                .bind(write.number.as_str())
                .fetch_optional(&mut *transaction)
                .await?;

        let locks = match existing_id {
            Some(id) => read_locks(&mut transaction, id).await?,
            None => BTreeSet::new(),
        };

        let effective = write.input.without_locked(&locks);
        let row = upsert_row(&mut transaction, write.number, &effective).await?;

        // 字段来源按字段合并: 锁定字段保留原来源, 其余字段采用本次命中的来源.
        let encoded = merge_sources(row.field_sources.as_deref(), &write.field_sources, &locks)?;
        let row = sqlx::query_as::<_, Metadata>(
            "UPDATE metadata SET field_sources = ?1 WHERE id = ?2 RETURNING *",
        )
        .bind(encoded)
        .bind(row.id)
        .fetch_one(&mut *transaction)
        .await?;

        transaction.commit().await?;

        // 只报告「本次有值但被锁挡住」的字段; 其余锁与本次刮削无关, 列出来会误导.
        let mut kept: Vec<LockableField> = locks
            .into_iter()
            .filter(|field| write.input.has_value(*field))
            .collect();
        kept.sort_unstable();

        Ok(ScrapeApplied { row, kept })
    }

    /// 某个番号条目被锁定的字段.
    pub async fn locks(
        &self,
        metadata_id: i64,
    ) -> Result<BTreeSet<LockableField>, RepositoryError> {
        let mut connection = self.database.read().acquire().await?;

        Ok(read_locks(&mut connection, metadata_id).await?)
    }

    /// 加锁或解锁, 返回操作后的锁定字段集合.
    pub async fn set_lock(
        &self,
        metadata_id: i64,
        field: LockableField,
        locked: bool,
    ) -> Result<BTreeSet<LockableField>, RepositoryError> {
        let mut transaction = self.database.write().begin().await?;

        if locked {
            sqlx::query(
                r#"
                INSERT INTO metadata_lock (metadata_id, field, created_at)
                VALUES (?1, ?2, ?3)
                ON CONFLICT(metadata_id, field) DO NOTHING
                "#,
            )
            .bind(metadata_id)
            .bind(field.as_str())
            .bind(Utc::now())
            .execute(&mut *transaction)
            .await?;
        } else {
            sqlx::query("DELETE FROM metadata_lock WHERE metadata_id = ?1 AND field = ?2")
                .bind(metadata_id)
                .bind(field.as_str())
                .execute(&mut *transaction)
                .await?;
        }

        let locks = read_locks(&mut transaction, metadata_id).await?;
        transaction.commit().await?;

        Ok(locks)
    }

    /// 用户标签, 按加入先后排序.
    pub async fn tags(&self, metadata_id: i64) -> Result<Vec<String>, RepositoryError> {
        let names = sqlx::query_scalar::<_, String>(
            r#"
            SELECT user_tag.name FROM user_tag
            JOIN metadata_tag ON metadata_tag.tag_id = user_tag.id
            WHERE metadata_tag.metadata_id = ?1
            ORDER BY user_tag.id ASC
            "#,
        )
        .bind(metadata_id)
        .fetch_all(self.database.read())
        .await?;

        Ok(names)
    }

    /// 添加标签. 同名标签复用同一条记录, 重复添加不会产生第二行.
    pub async fn add_tag(
        &self,
        metadata_id: i64,
        name: &str,
    ) -> Result<Vec<String>, RepositoryError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(RepositoryError::InvalidInput("标签不能为空".to_owned()));
        }

        let mut transaction = self.database.write().begin().await?;
        let now = Utc::now();
        let tag_id = sqlx::query_scalar::<_, i64>(
            r#"
            -- DO NOTHING 无法 RETURNING 既有行, 因此用一次空更新换 id.
            INSERT INTO user_tag (name, created_at) VALUES (?1, ?2)
            ON CONFLICT(name) DO UPDATE SET name = excluded.name
            RETURNING id
            "#,
        )
        .bind(name)
        .bind(now)
        .fetch_one(&mut *transaction)
        .await?;

        sqlx::query(
            "INSERT INTO metadata_tag (metadata_id, tag_id) VALUES (?1, ?2) ON CONFLICT DO NOTHING",
        )
        .bind(metadata_id)
        .bind(tag_id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;

        self.tags(metadata_id).await
    }

    /// 移除标签. 不再被任何番号条目引用的标签一并删除, 避免标签表堆积孤儿.
    pub async fn remove_tag(
        &self,
        metadata_id: i64,
        name: &str,
    ) -> Result<Vec<String>, RepositoryError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(RepositoryError::InvalidInput("标签不能为空".to_owned()));
        }

        let mut transaction = self.database.write().begin().await?;

        let removed = sqlx::query_scalar::<_, i64>(
            r#"
            DELETE FROM metadata_tag
            WHERE metadata_id = ?1
              AND tag_id = (SELECT id FROM user_tag WHERE name = ?2)
            RETURNING tag_id
            "#,
        )
        .bind(metadata_id)
        .bind(name)
        .fetch_optional(&mut *transaction)
        .await?;

        if let Some(tag_id) = removed {
            sqlx::query(
                "DELETE FROM user_tag WHERE id = ?1 AND NOT EXISTS (SELECT 1 FROM metadata_tag WHERE tag_id = ?1)",
            )
            .bind(tag_id)
            .execute(&mut *transaction)
            .await?;
        }

        transaction.commit().await?;

        self.tags(metadata_id).await
    }

    /// 手工修订字段. 被修订的字段同时加锁并将其来源记为 `manual`,
    /// 否则下一次刮削会把这些值覆盖掉.
    pub async fn update_fields(
        &self,
        number: &MediaNumber,
        patch: &MetadataPatch<'_>,
    ) -> Result<Metadata, RepositoryError> {
        let mut transaction = self.database.write().begin().await?;
        let existing: Option<(i64, Option<String>)> =
            sqlx::query_as("SELECT id, field_sources FROM metadata WHERE number = ?1")
                .bind(number.as_str())
                .fetch_optional(&mut *transaction)
                .await?;
        let Some((metadata_id, existing_sources)) = existing else {
            return Err(RepositoryError::NotFound);
        };

        let release_date = match patch.release_date {
            None => None,
            Some(raw) if raw.trim().is_empty() => Some(None),
            Some(raw) => Some(Some(
                NaiveDate::parse_from_str(raw.trim(), "%Y-%m-%d").map_err(|_| {
                    RepositoryError::InvalidInput(format!("上映日期应形如 2021-02-18: {raw}"))
                })?,
            )),
        };

        let mut touched: Vec<LockableField> = Vec::new();
        let mut builder = QueryBuilder::<Sqlite>::new("UPDATE metadata SET ");
        {
            let mut assignments = builder.separated(", ");

            if let Some(value) = patch.title {
                assignments
                    .push("title = ")
                    .push_bind_unseparated(blank_to_none(value));
                touched.push(LockableField::Title);
            }
            if let Some(value) = patch.plot {
                assignments
                    .push("plot = ")
                    .push_bind_unseparated(blank_to_none(value));
                touched.push(LockableField::Plot);
            }
            if let Some(date) = release_date {
                assignments
                    .push("release_date = ")
                    .push_bind_unseparated(date);
                touched.push(LockableField::ReleaseDate);
            }
            if let Some(minutes) = patch.runtime_minutes {
                assignments
                    .push("runtime_minutes = ")
                    .push_bind_unseparated(minutes);
                touched.push(LockableField::RuntimeMinutes);
            }
            if let Some(value) = patch.maker {
                assignments
                    .push("maker = ")
                    .push_bind_unseparated(blank_to_none(value));
                touched.push(LockableField::Maker);
            }
            if let Some(value) = patch.label {
                assignments
                    .push("label = ")
                    .push_bind_unseparated(blank_to_none(value));
                touched.push(LockableField::Label);
            }
            if let Some(value) = patch.series {
                assignments
                    .push("series = ")
                    .push_bind_unseparated(blank_to_none(value));
                touched.push(LockableField::Series);
            }
            if let Some(value) = patch.note {
                assignments
                    .push("note = ")
                    .push_bind_unseparated(blank_to_none(value));
            }
            assignments
                .push("updated_at = ")
                .push_bind_unseparated(Utc::now());
        }

        let mut row = builder
            .push(" WHERE number = ")
            .push_bind(number.as_str())
            .push(" RETURNING *")
            .build_query_as::<Metadata>()
            .fetch_one(&mut *transaction)
            .await?;

        if !touched.is_empty() {
            let updates: BTreeMap<String, String> = touched
                .iter()
                .map(|field| (field.as_str().to_owned(), MANUAL_SOURCE.to_owned()))
                .collect();
            for field in &touched {
                sqlx::query(
                    "INSERT INTO metadata_lock (metadata_id, field, created_at) VALUES (?1, ?2, ?3) ON CONFLICT(metadata_id, field) DO NOTHING",
                )
                .bind(metadata_id)
                .bind(field.as_str())
                .bind(Utc::now())
                .execute(&mut *transaction)
                .await?;
            }

            let encoded = merge_sources(existing_sources.as_deref(), &updates, &BTreeSet::new())?;
            row = sqlx::query_as::<_, Metadata>(
                "UPDATE metadata SET field_sources = ?1 WHERE id = ?2 RETURNING *",
            )
            .bind(encoded)
            .bind(metadata_id)
            .fetch_one(&mut *transaction)
            .await?;
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
    use std::collections::BTreeMap;

    use super::{
        MANUAL_SOURCE, Metadata, MetadataFilter, MetadataInput, MetadataPatch, MetadataRepository,
        ScrapeWrite,
    };
    use crate::content_type::ContentType;
    use crate::db::Database;
    use crate::lockable::LockableField;
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

    /// 一次刮削写入, 只带标题与来源.
    fn scrape<'a>(number: &'a MediaNumber, title: &'a str) -> ScrapeWrite<'a> {
        ScrapeWrite {
            number,
            input: MetadataInput {
                title: Some(title),
                ..MetadataInput::default()
            },
            field_sources: BTreeMap::from([("title".to_owned(), "dmm".to_owned())]),
        }
    }

    async fn seeded_row(repository: &MetadataRepository, title: &str) -> Metadata {
        let number = MediaNumber::parse("ABC-001").expect("应解析成功");
        repository
            .upsert(&number, &input(Some(title), &[]))
            .await
            .expect("应写入成功")
    }

    #[tokio::test]
    async fn locked_field_survives_rescrape() {
        let (_dir, repository) = repository().await;
        let number = MediaNumber::parse("ABC-001").expect("应解析成功");
        let row = seeded_row(&repository, "人工标题").await;
        repository
            .set_lock(row.id, LockableField::Title, true)
            .await
            .expect("应加锁成功");

        let applied = repository
            .apply_scraped(&scrape(&number, "刮削标题"))
            .await
            .expect("应写入成功");

        assert_eq!(applied.row.title.as_deref(), Some("人工标题"));
        assert_eq!(
            applied.kept,
            vec![LockableField::Title],
            "应报告被跳过的字段"
        );
    }

    #[tokio::test]
    async fn unlocked_field_is_overwritten() {
        let (_dir, repository) = repository().await;
        let number = MediaNumber::parse("ABC-001").expect("应解析成功");
        let row = seeded_row(&repository, "人工标题").await;
        repository
            .set_lock(row.id, LockableField::Title, true)
            .await
            .expect("应加锁成功");
        repository
            .apply_scraped(&scrape(&number, "第一次刮削"))
            .await
            .expect("应写入成功");

        repository
            .set_lock(row.id, LockableField::Title, false)
            .await
            .expect("应解锁成功");
        let applied = repository
            .apply_scraped(&scrape(&number, "第二次刮削"))
            .await
            .expect("应写入成功");

        assert_eq!(
            applied.row.title.as_deref(),
            Some("第二次刮削"),
            "解锁后应被覆盖"
        );
        assert!(applied.kept.is_empty());
    }

    #[tokio::test]
    async fn manual_edit_locks_field_and_marks_source() {
        let (_dir, repository) = repository().await;
        let number = MediaNumber::parse("ABC-001").expect("应解析成功");
        seeded_row(&repository, "原标题").await;

        let row = repository
            .update_fields(
                &number,
                &MetadataPatch {
                    title: Some("人工标题"),
                    note: Some("记一笔"),
                    ..MetadataPatch::default()
                },
            )
            .await
            .expect("应修订成功");

        assert_eq!(row.title.as_deref(), Some("人工标题"));
        assert_eq!(row.note.as_deref(), Some("记一笔"));

        let locks = repository.locks(row.id).await.expect("应查询成功");
        assert!(locks.contains(&LockableField::Title), "手工修订应同时加锁");
        let sources: BTreeMap<String, String> = row
            .field_sources
            .as_deref()
            .map(|raw| serde_json::from_str(raw).expect("应为 JSON"))
            .expect("应有字段来源");
        assert_eq!(
            sources.get("title").map(String::as_str),
            Some(MANUAL_SOURCE),
            "手工修订应把来源记为人工"
        );

        let applied = repository
            .apply_scraped(&scrape(&number, "刮削标题"))
            .await
            .expect("应写入成功");
        assert_eq!(
            applied.row.title.as_deref(),
            Some("人工标题"),
            "重刮不应覆盖人工值"
        );
        let sources: BTreeMap<String, String> = applied
            .row
            .field_sources
            .as_deref()
            .map(|raw| serde_json::from_str(raw).expect("应为 JSON"))
            .expect("应有字段来源");
        assert_eq!(
            sources.get("title").map(String::as_str),
            Some(MANUAL_SOURCE),
            "锁定字段的来源不应被刮削改写"
        );
    }

    #[tokio::test]
    async fn empty_value_clears_field() {
        let (_dir, repository) = repository().await;
        let number = MediaNumber::parse("ABC-001").expect("应解析成功");
        seeded_row(&repository, "原标题").await;

        let row = repository
            .update_fields(
                &number,
                &MetadataPatch {
                    title: Some("   "),
                    ..MetadataPatch::default()
                },
            )
            .await
            .expect("应修订成功");

        assert_eq!(row.title, None, "空白输入表示清空");
    }

    #[tokio::test]
    async fn rejects_malformed_release_date() {
        let (_dir, repository) = repository().await;
        let number = MediaNumber::parse("ABC-001").expect("应解析成功");
        seeded_row(&repository, "标题").await;

        let result = repository
            .update_fields(
                &number,
                &MetadataPatch {
                    release_date: Some("2021/02/18"),
                    ..MetadataPatch::default()
                },
            )
            .await;

        assert!(
            matches!(result, Err(super::RepositoryError::InvalidInput(_))),
            "日期格式不合法时应拒绝"
        );
    }

    #[tokio::test]
    async fn updating_absent_number_reports_not_found() {
        let (_dir, repository) = repository().await;
        let number = MediaNumber::parse("ABC-999").expect("应解析成功");

        let result = repository
            .update_fields(
                &number,
                &MetadataPatch {
                    title: Some("标题"),
                    ..MetadataPatch::default()
                },
            )
            .await;

        assert!(matches!(result, Err(super::RepositoryError::NotFound)));
    }

    #[tokio::test]
    async fn repeated_tag_is_stored_once() {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let database = crate::db::Database::open(&dir.path().join("amane.db"))
            .await
            .expect("应打开数据库");
        let repository = MetadataRepository::new(database.clone());
        let number = MediaNumber::parse("ABC-001").expect("应解析成功");
        let row = repository
            .upsert(&number, &input(Some("标题"), &[]))
            .await
            .expect("应写入成功");

        repository
            .add_tag(row.id, "收藏")
            .await
            .expect("应添加成功");
        let tags = repository
            .add_tag(row.id, " 收藏 ")
            .await
            .expect("重复添加不应失败");

        assert_eq!(tags, vec!["收藏"], "同名标签只保留一条");
        let distinct: i64 = sqlx::query_scalar("SELECT count(*) FROM user_tag")
            .fetch_one(database.read())
            .await
            .expect("应统计成功");
        assert_eq!(distinct, 1, "标签表不应出现同名第二行");
    }

    #[tokio::test]
    async fn removing_tag_drops_unused_name() {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let database = crate::db::Database::open(&dir.path().join("amane.db"))
            .await
            .expect("应打开数据库");
        let repository = MetadataRepository::new(database.clone());
        let number = MediaNumber::parse("ABC-001").expect("应解析成功");
        let row = repository
            .upsert(&number, &input(Some("标题"), &[]))
            .await
            .expect("应写入成功");
        repository
            .add_tag(row.id, "待删")
            .await
            .expect("应添加成功");

        let tags = repository
            .remove_tag(row.id, "待删")
            .await
            .expect("应移除成功");

        assert!(tags.is_empty());
        let remaining: i64 = sqlx::query_scalar("SELECT count(*) FROM user_tag")
            .fetch_one(database.read())
            .await
            .expect("应统计成功");
        assert_eq!(remaining, 0, "无人引用的标签应一并删除");
    }

    #[tokio::test]
    async fn rejects_blank_tag() {
        let (_dir, repository) = repository().await;
        let row = seeded_row(&repository, "标题").await;

        for name in ["", "   "] {
            let added = repository.add_tag(row.id, name).await;
            let removed = repository.remove_tag(row.id, name).await;
            assert!(
                matches!(added, Err(super::RepositoryError::InvalidInput(_))),
                "添加 {name:?} 应被拒绝"
            );
            assert!(
                matches!(removed, Err(super::RepositoryError::InvalidInput(_))),
                "移除 {name:?} 应被拒绝"
            );
        }
    }

    #[tokio::test]
    async fn kept_reports_only_fields_with_incoming_values() {
        let (_dir, repository) = repository().await;
        let number = MediaNumber::parse("ABC-001").expect("应解析成功");
        let row = seeded_row(&repository, "人工标题").await;
        // 两个字段都锁, 但本次刮削只带回标题: 只有标题算「被锁挡住」.
        repository
            .set_lock(row.id, LockableField::Title, true)
            .await
            .expect("应加锁成功");
        repository
            .set_lock(row.id, LockableField::Series, true)
            .await
            .expect("应加锁成功");
        let mut write = scrape(&number, "刮削标题");
        write.input.plot = Some("刮削简介");

        let applied = repository.apply_scraped(&write).await.expect("应写入成功");

        assert_eq!(
            applied.kept,
            vec![LockableField::Title],
            "只有本次带来值的字段才算保留"
        );
        assert_eq!(
            applied.row.plot.as_deref(),
            Some("刮削简介"),
            "未锁的简介应写入"
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

        let rows = repository
            .list(&MetadataFilter::default(), 10, 0)
            .await
            .expect("应列出成功");
        let numbers: Vec<&str> = rows.iter().map(|row| row.number.as_str()).collect();

        assert_eq!(numbers, ["ABC-003", "ABC-002", "ABC-001"]);
    }
}
