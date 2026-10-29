use std::collections::{BTreeMap, BTreeSet};

mod field;
mod removal;

pub use field::{FIELDS, FieldSpec, FieldValue};
pub use removal::{MetadataRemoval, ResourceFiles};

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::types::Json;
use sqlx::{FromRow, QueryBuilder, Sqlite, SqliteConnection};
use thiserror::Error;

use crate::content_type::ContentType;
use crate::db::Database;
use crate::lockable::LockableField;
use crate::number::MediaNumber;
use crate::resource::Resource;

/// 列表筛选条件. 各项为空表示不限制, 组合时取交集.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MetadataFilter {
    pub content_type: Option<ContentType>,
    /// 贡献过该番号条目的来源标识.
    pub source: Option<String>,
    /// 关键字: 匹配番号或标题, 忽略大小写.
    pub keyword: Option<String>,
    /// 题材名; 命中该题材的条目.
    pub genre: Option<String>,
}

impl MetadataFilter {
    /// 校验取值域有界的筛选值: 来源必须在已知集合里.
    ///
    /// 题材是开放取值, 不在此列 —— 写错一个字得到空列表, 而不是让整个筛选报错.
    pub fn validate(&self, known_sources: &[&str]) -> Result<(), FilterError> {
        let Some(source) = self.source.as_deref() else {
            return Ok(());
        };
        if known_sources.contains(&source) {
            return Ok(());
        }

        Err(FilterError::UnknownSource(source.to_owned()))
    }

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
    /// 竖版海报的远端地址; 首个用于详情页与资源物化.
    pub poster_urls: Option<Json<Vec<String>>>,
    /// 横版图的远端地址.
    pub thumb_urls: Option<Json<Vec<String>>>,
    /// 预告片的远端地址.
    pub trailer_urls: Option<Json<Vec<String>>>,
    /// 剧照, 按站点分组; 只存地址, 不物化.
    pub extrafanart_urls: Option<Json<BTreeMap<String, Vec<String>>>>,
    /// 各站评分.
    pub scores: Option<Json<BTreeMap<String, f64>>>,
    /// 各站的外部标识.
    pub external_ids: Option<Json<BTreeMap<String, String>>>,
    /// 各站的原站地址.
    pub source_urls: Option<Json<BTreeMap<String, String>>>,
    /// 海报存到本地后的资源标识; 为 `None` 时详情页只能引用远端地址.
    pub poster_resource_id: Option<i64>,
    /// 横版图存到本地后的资源标识.
    pub cover_resource_id: Option<i64>,
    /// 字段来源表; 键是字段名, 值是来源标识.
    pub field_sources: Option<Json<FieldSources>>,
    /// 题材. 按来源整份取首个非空, 不混拼.
    pub genres: Option<Json<Genres>>,
    /// 用户备注, 只有人工写入.
    pub note: Option<String>,
    pub content_type: ContentType,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Metadata {
    /// 竖版海报的首项. 单值不落库, 详情页与资源物化都从这里取.
    pub fn poster_url(&self) -> Option<&str> {
        self.poster_urls
            .as_ref()
            .and_then(|urls| urls.0.first())
            .map(String::as_str)
    }

    /// 横版图的首项.
    pub fn thumb_url(&self) -> Option<&str> {
        self.thumb_urls
            .as_ref()
            .and_then(|urls| urls.0.first())
            .map(String::as_str)
    }
}

/// 字段来源表: 键是字段名, 值是贡献该字段的来源标识.
///
/// 单独成类型而不是裸 `BTreeMap`: 落库形态与人工修订标记 (`manual`) 都挂在这里,
/// 读写两端拿到的始终是同一份结构.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FieldSources(BTreeMap<String, String>);

impl FieldSources {
    pub fn get(&self, field: &str) -> Option<&String> {
        self.0.get(field)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// 写入一个字段的来源; 被锁定的字段保持原来源.
    fn merge(&mut self, updates: &BTreeMap<String, String>, locked: &BTreeSet<LockableField>) {
        for (field, source) in updates {
            let blocked = LockableField::parse(field).is_some_and(|field| locked.contains(&field));
            if !blocked {
                self.0.insert(field.clone(), source.clone());
            }
        }
    }

    /// 按字段读取来源; 供详情视图逐字段展示.
    pub fn to_map(&self) -> BTreeMap<String, String> {
        self.0.clone()
    }
}

impl From<BTreeMap<String, String>> for FieldSources {
    fn from(value: BTreeMap<String, String>) -> Self {
        Self(value)
    }
}

/// 题材列表.
///
/// 与演员分开: 题材没有顺序语义, 也不参与逐项修订, 存成一列 JSON 数组即可.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Genres(Vec<String>);

impl Genres {
    pub fn as_slice(&self) -> &[String] {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<Vec<String>> for Genres {
    fn from(value: Vec<String>) -> Self {
        Self(value)
    }
}

impl From<&[String]> for Genres {
    fn from(value: &[String]) -> Self {
        Self(value.to_vec())
    }
}

/// 一个来源在某一时刻返回的元数据快照.
///
/// 存的是来源自己的结果, 不是聚合后的条目: 字段溯源要能回答「当时这个来源说了什么」.
/// 缺字段按默认值读取, 旧载荷因此不会因为新增字段而读不出来.
// 评分是浮点, 因此不派生 `Eq`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct ScrapedMetadata {
    pub title: Option<String>,
    pub plot: Option<String>,
    pub release_date: Option<NaiveDate>,
    pub runtime_minutes: Option<i32>,
    pub maker: Option<String>,
    pub label: Option<String>,
    pub series: Option<String>,
    pub directors: Vec<String>,
    pub actors: Vec<String>,
    /// 旧载荷把这一项写作 `tags`, 读入时一并接受.
    #[serde(alias = "tags")]
    pub genres: Vec<String>,
    pub poster_urls: Vec<String>,
    /// 横版图; 与竖版海报分开, 详情页两个位置各用一张.
    pub thumb_urls: Vec<String>,
    pub trailer_urls: Vec<String>,
    /// 剧照, 按站点分组.
    pub extrafanart_urls: BTreeMap<String, Vec<String>>,
    /// 各站评分.
    pub scores: BTreeMap<String, f64>,
    /// 各站的外部标识.
    pub external_ids: BTreeMap<String, String>,
    /// 各站的原站地址.
    pub source_urls: BTreeMap<String, String>,
}

/// 一条分站快照行.
#[derive(Debug, Clone, FromRow)]
pub struct SourceSnapshot {
    pub source_id: String,
    pub payload: Json<ScrapedMetadata>,
    pub fetched_at: DateTime<Utc>,
}

/// 待写入的一条分站快照.
#[derive(Debug, Clone)]
pub struct SourceSnapshotInput<'a> {
    pub source_id: &'a str,
    pub payload: ScrapedMetadata,
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
    /// 竖版海报的远端地址; 空列表按「本次没有」处理.
    pub poster_urls: Option<&'a [String]>,
    /// 横版图的远端地址.
    pub thumb_urls: Option<&'a [String]>,
    pub trailer_urls: Option<&'a [String]>,
    pub extrafanart_urls: Option<&'a BTreeMap<String, Vec<String>>>,
    pub scores: Option<&'a BTreeMap<String, f64>>,
    pub external_ids: Option<&'a BTreeMap<String, String>>,
    pub source_urls: Option<&'a BTreeMap<String, String>>,
    /// `None` 表示本次不改动题材; `Some` 表示整体替换, 空列表即清空.
    pub genres: Option<&'a [String]>,
    /// `None` 表示本次不改动演员名单; `Some` 表示整体替换, 空列表即清空.
    pub actors: Option<&'a [String]>,
}

impl MetadataInput<'_> {
    /// 去掉被锁定字段后的输入, 以及因此被丢弃的字段.
    ///
    /// 返回的字段正是「本次有值但被锁挡住」的那些, 供任务日志说明跳过原因.
    pub fn without_locked(&self, locks: &BTreeSet<LockableField>) -> (Self, Vec<LockableField>) {
        let mut effective = self.clone();
        let mut dropped = Vec::new();

        for spec in FIELDS {
            if !locks.contains(&spec.field) {
                continue;
            }
            if (spec.clear)(&mut effective) {
                dropped.push(spec.field);
            }
        }
        dropped.sort_unstable();

        (effective, dropped)
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
    existing: Option<&FieldSources>,
    updates: &BTreeMap<String, String>,
    locked: &BTreeSet<LockableField>,
) -> FieldSources {
    let mut merged = existing.cloned().unwrap_or_default();
    merged.merge(updates, locked);
    merged
}

/// 写入或更新一行, 调用方负责事务边界.
///
/// 列清单、取值与 `COALESCE` 都从字段表生成: 加字段不必改这段 SQL, 也不会出现
/// 「列顺序与绑定顺序对不上」这类静默错位.
async fn upsert_row(
    transaction: &mut sqlx::Transaction<'_, Sqlite>,
    number: &MediaNumber,
    input: &MetadataInput<'_>,
) -> Result<Metadata, sqlx::Error> {
    let now = Utc::now();

    let mut builder = QueryBuilder::<Sqlite>::new("INSERT INTO metadata (number");
    for spec in FIELDS {
        builder.push(", ").push(spec.column);
    }
    builder.push(
        ", thumb_urls, trailer_urls, extrafanart_urls, scores, external_ids, source_urls, \
         genres, content_type, created_at, updated_at) VALUES (",
    );
    builder.push_bind(number.as_str());
    for spec in FIELDS {
        builder.push(", ");
        push_value(&mut builder, (spec.take)(input));
    }
    builder
        .push(", ")
        .push_bind(input.thumb_urls.map(|urls| Json(urls.to_vec())))
        .push(", ")
        .push_bind(input.trailer_urls.map(|urls| Json(urls.to_vec())))
        .push(", ")
        .push_bind(input.extrafanart_urls.map(|map| Json(map.clone())))
        .push(", ")
        .push_bind(input.scores.map(|map| Json(map.clone())))
        .push(", ")
        .push_bind(input.external_ids.map(|map| Json(map.clone())))
        .push(", ")
        .push_bind(input.source_urls.map(|map| Json(map.clone())))
        .push(", ")
        .push_bind(input.genres.map(|values| Json(Genres::from(values))))
        .push(", ")
        .push_bind(number.content_type())
        .push(", ")
        .push_bind(now)
        .push(", ")
        .push_bind(now)
        .push(") ON CONFLICT(number) DO UPDATE SET ");

    for spec in FIELDS {
        let column = spec.column;
        builder
            .push(column)
            .push(" = COALESCE(excluded.")
            .push(column)
            .push(", metadata.")
            .push(column)
            .push("), ");
    }
    builder
        .push("thumb_urls = COALESCE(excluded.thumb_urls, metadata.thumb_urls), ")
        .push("trailer_urls = COALESCE(excluded.trailer_urls, metadata.trailer_urls), ")
        .push("extrafanart_urls = COALESCE(excluded.extrafanart_urls, metadata.extrafanart_urls), ")
        .push("scores = COALESCE(excluded.scores, metadata.scores), ")
        .push("external_ids = COALESCE(excluded.external_ids, metadata.external_ids), ")
        .push("source_urls = COALESCE(excluded.source_urls, metadata.source_urls), ")
        .push("genres = COALESCE(excluded.genres, metadata.genres), ")
        .push("content_type = excluded.content_type, updated_at = excluded.updated_at RETURNING *");

    let row = builder
        .build_query_as::<Metadata>()
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

/// 把一个字段取值绑定到查询; 三种列各有自己的 SQL 类型, 分派只在这里发生一次.
fn push_value(builder: &mut QueryBuilder<'_, Sqlite>, value: Option<FieldValue>) {
    match value {
        Some(FieldValue::Text(text)) => {
            builder.push_bind(blank_to_none(&text).map(str::to_owned));
        }
        Some(FieldValue::Date(date)) => {
            builder.push_bind(date);
        }
        Some(FieldValue::Number(minutes)) => {
            builder.push_bind(minutes);
        }
        Some(FieldValue::List(values)) => {
            builder.push_bind(Json(values));
        }
        // 来源没有给出这一项: 写 NULL, 由 `COALESCE` 保留既有值.
        None => {
            builder.push_bind(Option::<String>::None);
        }
    }
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

    if let Some(genre) = filter.genre.as_deref() {
        builder
            .push(" AND EXISTS (SELECT 1 FROM json_each(metadata.genres) WHERE json_each.value = ")
            .push_bind(genre.to_owned())
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

/// 筛选条件不合法的原因. 与仓储错误分开: 它不是持久化失败, 也不需要日志.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum FilterError {
    #[error("未知来源: {0}")]
    UnknownSource(String),
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
    /// 创建或更新条目, 跳过被锁定的字段.
    ///
    /// 所有写入口都要过锁: 留一条不过锁的路径, 人工锁定的值就会被一次无关的写入冲掉.
    pub async fn upsert(
        &self,
        number: &MediaNumber,
        input: &MetadataInput<'_>,
    ) -> Result<Metadata, RepositoryError> {
        let mut transaction = self.database.write().begin().await?;

        let existing_id: Option<i64> =
            sqlx::query_scalar("SELECT id FROM metadata WHERE number = ?1")
                .bind(number.as_str())
                .fetch_optional(&mut *transaction)
                .await?;
        let locks = match existing_id {
            Some(id) => read_locks(&mut transaction, id).await?,
            None => BTreeSet::new(),
        };

        // 被丢弃的字段在这里不需要报告: 调用方没有「本次刮到了什么」的上下文.
        let (effective, _dropped) = input.without_locked(&locks);
        let row = upsert_row(&mut transaction, number, &effective).await?;
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

        // 只报告「本次有值但被锁挡住」的字段; 其余锁与本次刮削无关, 列出来会误导.
        let (effective, kept) = write.input.without_locked(&locks);
        let row = upsert_row(&mut transaction, write.number, &effective).await?;

        // 字段来源按字段合并: 锁定字段保留原来源, 其余字段采用本次命中的来源.
        let encoded = merge_sources(
            row.field_sources.as_ref().map(|json| &json.0),
            &write.field_sources,
            &locks,
        );
        let row = sqlx::query_as::<_, Metadata>(
            "UPDATE metadata SET field_sources = ?1 WHERE id = ?2 RETURNING *",
        )
        .bind(Json(&encoded))
        .bind(row.id)
        .fetch_one(&mut *transaction)
        .await?;

        transaction.commit().await?;

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
        let existing: Option<(i64, Option<Json<FieldSources>>)> =
            sqlx::query_as("SELECT id, field_sources FROM metadata WHERE number = ?1")
                .bind(number.as_str())
                .fetch_optional(&mut *transaction)
                .await?;
        let Some((metadata_id, existing_sources)) = existing else {
            return Err(RepositoryError::NotFound);
        };

        // 列名与取值都取自字段表: 写列与登记 `touched` 由同一个循环产生, 不会一边写列一边漏记字段.
        let mut touched: Vec<LockableField> = Vec::new();
        let mut builder = QueryBuilder::<Sqlite>::new("UPDATE metadata SET ");
        {
            let mut assignments = builder.separated(", ");

            for spec in FIELDS {
                let Some(value) = (spec.patch)(patch)? else {
                    continue;
                };
                match value {
                    FieldValue::Text(text) => {
                        assignments
                            .push(spec.column)
                            .push_unseparated(" = ")
                            .push_bind_unseparated(blank_to_none(&text).map(str::to_owned));
                    }
                    FieldValue::Date(date) => {
                        assignments
                            .push(spec.column)
                            .push_unseparated(" = ")
                            .push_bind_unseparated(date);
                    }
                    FieldValue::Number(minutes) => {
                        assignments
                            .push(spec.column)
                            .push_unseparated(" = ")
                            .push_bind_unseparated(minutes);
                    }
                    FieldValue::List(values) => {
                        assignments
                            .push(spec.column)
                            .push_unseparated(" = ")
                            .push_bind_unseparated(Json(values));
                    }
                }
                touched.push(spec.field);
            }

            // 备注可写但不可锁, 不属于刮削字段, 因此不进字段表.
            if let Some(note) = patch.note {
                assignments
                    .push("note")
                    .push_unseparated(" = ")
                    .push_bind_unseparated(blank_to_none(note));
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

            let encoded = merge_sources(
                existing_sources.as_ref().map(|json| &json.0),
                &updates,
                &BTreeSet::new(),
            );
            row = sqlx::query_as::<_, Metadata>(
                "UPDATE metadata SET field_sources = ?1 WHERE id = ?2 RETURNING *",
            )
            .bind(Json(&encoded))
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
        snapshots: &[SourceSnapshotInput<'_>],
    ) -> Result<(), RepositoryError> {
        let mut transaction = self.database.write().begin().await?;
        let now = Utc::now();

        sqlx::query("DELETE FROM source_snapshot WHERE metadata_id = ?1")
            .bind(metadata_id)
            .execute(&mut *transaction)
            .await?;

        for snapshot in snapshots {
            sqlx::query(
                "INSERT INTO source_snapshot (metadata_id, source_id, payload, fetched_at) VALUES (?1, ?2, ?3, ?4)",
            )
            .bind(metadata_id)
            .bind(snapshot.source_id)
            .bind(Json(&snapshot.payload))
            .bind(now)
            .execute(&mut *transaction)
            .await?;
        }

        transaction.commit().await?;
        Ok(())
    }

    /// 全部出现过的题材, 按名称排序. 筛选下拉框据此给出可选项.
    pub async fn genre_options(&self) -> Result<Vec<String>, RepositoryError> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT DISTINCT json_each.value FROM metadata, json_each(metadata.genres) \
             ORDER BY json_each.value ASC",
        )
        .fetch_all(self.database.read())
        .await?;

        Ok(rows.into_iter().map(|(value,)| value).collect())
    }

    /// 删除条目与全部关联数据, 返回不再被引用的资源.
    ///
    /// 只处理数据库: 外部请用 [`MetadataRemoval`], 它按「数据库先行、文件随后」的顺序
    /// 把文件也清掉. 这里收窄为 `pub(crate)`, 绕过清理的入口从类型上就不存在.
    pub(crate) async fn delete(
        &self,
        number: &MediaNumber,
    ) -> Result<Vec<Resource>, RepositoryError> {
        let mut transaction = self.database.write().begin().await?;

        let existing: Option<(i64, Option<i64>, Option<i64>)> = sqlx::query_as(
            "SELECT id, poster_resource_id, cover_resource_id FROM metadata WHERE number = ?1",
        )
        .bind(number.as_str())
        .fetch_optional(&mut *transaction)
        .await?;
        let Some((id, poster_resource_id, cover_resource_id)) = existing else {
            return Err(RepositoryError::NotFound);
        };

        // 子表由外键级联删除, 这里只删主行.
        sqlx::query("DELETE FROM metadata WHERE id = ?1")
            .bind(id)
            .execute(&mut *transaction)
            .await?;

        // 资源按远端地址共享: 还有别的条目引用时保留, 否则连同登记一起删.
        // 两类图各占一列, 同一地址可能被两列同时引用, 因此按标识去重后再判.
        let mut candidates: Vec<i64> = [poster_resource_id, cover_resource_id]
            .into_iter()
            .flatten()
            .collect();
        candidates.sort_unstable();
        candidates.dedup();

        let mut resources = Vec::new();
        for resource_id in candidates {
            let still_used: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM metadata \
                 WHERE poster_resource_id = ?1 OR cover_resource_id = ?1",
            )
            .bind(resource_id)
            .fetch_one(&mut *transaction)
            .await?;
            if still_used > 0 {
                continue;
            }
            if let Some(resource) =
                sqlx::query_as::<_, Resource>("DELETE FROM resource WHERE id = ?1 RETURNING *")
                    .bind(resource_id)
                    .fetch_optional(&mut *transaction)
                    .await?
            {
                resources.push(resource);
            }
        }

        transaction.commit().await?;

        Ok(resources)
    }

    /// 番号条目的各来源快照, 按来源标识排序.
    pub async fn snapshots(
        &self,
        metadata_id: i64,
    ) -> Result<Vec<SourceSnapshot>, RepositoryError> {
        let rows = sqlx::query_as::<_, SourceSnapshot>(
            "SELECT source_id, payload, fetched_at FROM source_snapshot \
             WHERE metadata_id = ?1 ORDER BY source_id ASC",
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

    /// 把条目的封面指向某条资源.
    pub async fn set_cover_resource(
        &self,
        metadata_id: i64,
        resource_id: i64,
    ) -> Result<(), RepositoryError> {
        sqlx::query("UPDATE metadata SET cover_resource_id = ?1 WHERE id = ?2")
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
        FilterError, MANUAL_SOURCE, Metadata, MetadataFilter, MetadataInput, MetadataPatch,
        MetadataRepository, RepositoryError, ScrapeWrite, ScrapedMetadata, SourceSnapshotInput,
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

    /// 同时拿到数据库句柄, 供需要绕过仓储写入原始内容的用例使用.
    async fn repository_with_database() -> (tempfile::TempDir, MetadataRepository, Database) {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let database = Database::open(&dir.path().join("amane.db"))
            .await
            .expect("应打开数据库");
        (dir, MetadataRepository::new(database.clone()), database)
    }

    fn input<'a>(title: Option<&'a str>, actors: &'a [String]) -> MetadataInput<'a> {
        MetadataInput {
            title,
            actors: Some(actors),
            ..MetadataInput::default()
        }
    }

    #[tokio::test]
    async fn delete_removes_entry_and_related_rows() {
        let (_dir, repository, database) = repository_with_database().await;
        let number = MediaNumber::parse("ABC-001").expect("应解析成功");
        let actors = vec!["演员".to_owned()];
        let row = repository
            .upsert(
                &number,
                &MetadataInput {
                    title: Some("标题"),
                    actors: Some(&actors),
                    ..MetadataInput::default()
                },
            )
            .await
            .expect("应写入成功");
        repository
            .set_lock(row.id, LockableField::Title, true)
            .await
            .expect("应加锁成功");
        repository
            .add_tag(row.id, "收藏")
            .await
            .expect("应加标签成功");
        repository
            .replace_snapshots(
                row.id,
                &[SourceSnapshotInput {
                    source_id: "dmm",
                    payload: ScrapedMetadata::default(),
                }],
            )
            .await
            .expect("应写入快照");

        let orphans = repository.delete(&number).await.expect("应删除成功");

        assert!(orphans.is_empty(), "没有关联资源");
        assert!(
            repository
                .find(&number)
                .await
                .expect("应查询成功")
                .is_none(),
            "条目应已删除"
        );
        for table in [
            "metadata_actor",
            "metadata_lock",
            "metadata_tag",
            "source_snapshot",
        ] {
            let left: i64 = sqlx::query_scalar(&format!(
                "SELECT COUNT(*) FROM {table} WHERE metadata_id = ?1"
            ))
            .bind(row.id)
            .fetch_one(database.read())
            .await
            .expect("应查询成功");
            assert_eq!(left, 0, "{table} 不应留下孤儿行");
        }
    }

    #[tokio::test]
    async fn delete_removes_entry_without_related_data() {
        let (_dir, repository) = repository().await;
        let number = MediaNumber::parse("ABC-002").expect("应解析成功");
        repository
            .upsert(&number, &MetadataInput::default())
            .await
            .expect("应写入成功");

        let orphans = repository.delete(&number).await.expect("应删除成功");

        assert!(orphans.is_empty());
        assert!(
            repository
                .find(&number)
                .await
                .expect("应查询成功")
                .is_none()
        );
    }

    #[tokio::test]
    async fn delete_returns_resource_that_nobody_else_uses() {
        let (_dir, repository, database) = repository_with_database().await;
        let resources = crate::resource::ResourceRepository::new(database);
        let number = MediaNumber::parse("ABC-003").expect("应解析成功");
        let row = repository
            .upsert(&number, &MetadataInput::default())
            .await
            .expect("应写入成功");
        let resource = resources
            .reserve("https://example.invalid/p.jpg", "image/jpeg", 1)
            .await
            .expect("应登记成功");
        resources
            .link_poster(row.id, Some(resource.id))
            .await
            .expect("应关联成功");

        let orphans = repository.delete(&number).await.expect("应删除成功");

        assert_eq!(orphans.len(), 1, "无主资源应交给调用方清理");
        assert!(
            resources
                .find(resource.id)
                .await
                .expect("应查询成功")
                .is_none(),
            "资源登记应随条目删除"
        );
    }

    #[tokio::test]
    async fn delete_keeps_resource_still_used_by_another_entry() {
        let (_dir, repository, database) = repository_with_database().await;
        let resources = crate::resource::ResourceRepository::new(database);
        let shared = resources
            .reserve("https://example.invalid/shared.jpg", "image/jpeg", 1)
            .await
            .expect("应登记成功");
        for raw in ["ABC-004", "ABC-005"] {
            let number = MediaNumber::parse(raw).expect("应解析成功");
            let row = repository
                .upsert(&number, &MetadataInput::default())
                .await
                .expect("应写入成功");
            resources
                .link_poster(row.id, Some(shared.id))
                .await
                .expect("应关联成功");
        }

        let number = MediaNumber::parse("ABC-004").expect("应解析成功");
        let orphans = repository.delete(&number).await.expect("应删除成功");

        assert!(orphans.is_empty(), "另一个条目还在引用, 资源不能删");
        assert!(
            resources
                .find(shared.id)
                .await
                .expect("应查询成功")
                .is_some()
        );
    }

    #[tokio::test]
    async fn delete_recycles_cover_resource_too() {
        let (_dir, repository, database) = repository_with_database().await;
        let resources = crate::resource::ResourceRepository::new(database);
        let number = MediaNumber::parse("ABC-006").expect("应解析成功");
        let row = repository
            .upsert(&number, &MetadataInput::default())
            .await
            .expect("应写入成功");
        let cover = resources
            .reserve("https://example.invalid/cover.jpg", "image/jpeg", 1)
            .await
            .expect("应登记成功");
        repository
            .set_cover_resource(row.id, cover.id)
            .await
            .expect("应关联成功");

        let orphans = repository.delete(&number).await.expect("应删除成功");

        assert_eq!(orphans.len(), 1, "封面资源也应交给调用方清理");
        assert!(
            resources
                .find(cover.id)
                .await
                .expect("应查询成功")
                .is_none()
        );
    }

    #[tokio::test]
    async fn reads_snapshot_payload_written_before_genres_rename() {
        let (_dir, repository, database) = repository_with_database().await;
        let number = MediaNumber::parse("ABC-007").expect("应解析成功");
        let row = repository
            .upsert(&number, &MetadataInput::default())
            .await
            .expect("应写入成功");
        // 旧版本的载荷把题材写作 `tags`, 也没有横版图; 读端必须仍能解出.
        sqlx::query(
            "INSERT INTO source_snapshot (metadata_id, source_id, payload, fetched_at) \
             VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(row.id)
        .bind("dmm")
        .bind(r#"{"title":"标题","tags":["ドラマ"]}"#)
        .bind(chrono::Utc::now())
        .execute(database.write())
        .await
        .expect("应写入旧载荷");

        let snapshots = repository.snapshots(row.id).await.expect("旧载荷应能读出");

        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].payload.0.genres, vec!["ドラマ"]);
        assert!(snapshots[0].payload.0.thumb_urls.is_empty());
    }

    #[tokio::test]
    async fn delete_absent_entry_is_not_found() {
        let (_dir, repository) = repository().await;
        let number = MediaNumber::parse("ZZZ-999").expect("应解析成功");

        let result = repository.delete(&number).await;

        assert!(matches!(result, Err(RepositoryError::NotFound)));
    }

    #[tokio::test]
    async fn every_text_field_is_locked_by_manual_edit() {
        let (_dir, repository) = repository().await;
        let number = MediaNumber::parse("ABC-008").expect("应解析成功");
        let row = repository
            .upsert(&number, &MetadataInput::default())
            .await
            .expect("应写入成功");

        repository
            .update_fields(
                &number,
                &MetadataPatch {
                    title: Some("标题"),
                    plot: Some("简介"),
                    maker: Some("片商"),
                    label: Some("厂牌"),
                    series: Some("系列"),
                    ..MetadataPatch::default()
                },
            )
            .await
            .expect("应修订成功");

        // 列清单与 `touched` 由同一个循环产生: 漏登记任何一个字段都会在这里暴露.
        let locks = repository.locks(row.id).await.expect("应查询成功");
        let sources = repository
            .find(&number)
            .await
            .expect("应查询成功")
            .expect("应存在")
            .field_sources
            .as_ref()
            .map(|json| json.0.to_map())
            .expect("应有字段来源");
        for field in [
            LockableField::Title,
            LockableField::Plot,
            LockableField::Maker,
            LockableField::Label,
            LockableField::Series,
        ] {
            assert!(locks.contains(&field), "{field} 应被加锁");
            assert_eq!(
                sources.get(field.as_str()).map(String::as_str),
                Some(MANUAL_SOURCE),
                "{field} 的来源应记为人工"
            );
        }
    }

    #[tokio::test]
    async fn patch_writes_every_column() {
        let (_dir, repository) = repository().await;
        let number = MediaNumber::parse("ABC-009").expect("应解析成功");
        repository
            .upsert(&number, &MetadataInput::default())
            .await
            .expect("应写入成功");

        // 数值列与文本列的赋值拼接不同, 漏一个分隔符就会拼出语法错误.
        let row = repository
            .update_fields(
                &number,
                &MetadataPatch {
                    title: Some("标题"),
                    runtime_minutes: Some(120),
                    note: Some("备注"),
                    ..MetadataPatch::default()
                },
            )
            .await
            .expect("应修订成功");

        assert_eq!(row.title.as_deref(), Some("标题"));
        assert_eq!(row.runtime_minutes, Some(120));
        assert_eq!(row.note.as_deref(), Some("备注"));
    }

    #[test]
    fn filter_rejects_unknown_source() {
        let filter = MetadataFilter {
            source: Some("nowhere".to_owned()),
            ..MetadataFilter::default()
        };

        assert_eq!(
            filter.validate(&["dmm", "javdb"]),
            Err(FilterError::UnknownSource("nowhere".to_owned()))
        );
        assert!(
            filter.validate(&["dmm", "nowhere"]).is_ok(),
            "已知来源应通过"
        );
        assert!(
            MetadataFilter::default().validate(&[]).is_ok(),
            "未指定来源时不校验"
        );
    }

    #[tokio::test]
    async fn list_filters_by_genre() {
        let (_dir, repository) = repository().await;
        let drama = vec!["ドラマ".to_owned()];
        let action = vec!["アクション".to_owned()];
        for (raw, genres) in [("ABC-001", &drama), ("ABC-002", &action)] {
            let number = MediaNumber::parse(raw).expect("应解析成功");
            repository
                .upsert(
                    &number,
                    &MetadataInput {
                        genres: Some(genres),
                        ..MetadataInput::default()
                    },
                )
                .await
                .expect("应写入成功");
        }

        let filter = MetadataFilter {
            genre: Some("ドラマ".to_owned()),
            ..MetadataFilter::default()
        };
        let rows = repository.list(&filter, 10, 0).await.expect("应列出成功");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].number, "ABC-001");
        assert_eq!(
            repository.genre_options().await.expect("应查询成功"),
            vec!["アクション".to_owned(), "ドラマ".to_owned()]
        );
    }

    #[tokio::test]
    async fn field_sources_round_trip_as_typed_map() {
        let (_dir, repository) = repository().await;
        let number = MediaNumber::parse("ABC-001").expect("应解析成功");
        repository
            .upsert(&number, &input(None, &[]))
            .await
            .expect("应写入成功");

        let applied = repository
            .apply_scraped(&scrape(&number, "标题"))
            .await
            .expect("应写入成功");
        let sources = applied
            .row
            .field_sources
            .as_ref()
            .map(|json| json.0.to_map())
            .expect("应有字段来源");

        assert_eq!(
            sources.get("title").map(String::as_str),
            Some("dmm"),
            "读回的应当是结构化的来源表, 不是待解析的文本"
        );
    }

    #[tokio::test]
    async fn reads_field_sources_written_as_plain_json() {
        let (_dir, repository, database) = repository_with_database().await;
        let number = MediaNumber::parse("ABC-001").expect("应解析成功");
        repository
            .upsert(&number, &input(None, &[]))
            .await
            .expect("应写入成功");
        // 早期版本直接把 JSON 文本写进列里; 读端必须仍能解出同一份结构.
        sqlx::query("UPDATE metadata SET field_sources = ?1 WHERE number = ?2")
            .bind(r#"{"title":"dmm","maker":"javdb"}"#)
            .bind(number.as_str())
            .execute(database.write())
            .await
            .expect("应写入原始文本");

        let row = repository
            .find(&number)
            .await
            .expect("应查询成功")
            .expect("应存在");
        let sources = row
            .field_sources
            .as_ref()
            .map(|json| json.0.to_map())
            .expect("应有字段来源");

        assert_eq!(sources.get("title").map(String::as_str), Some("dmm"));
        assert_eq!(sources.get("maker").map(String::as_str), Some("javdb"));
    }

    #[tokio::test]
    async fn corrupted_field_sources_surface_as_error() {
        let (_dir, repository, database) = repository_with_database().await;
        let number = MediaNumber::parse("ABC-001").expect("应解析成功");
        repository
            .upsert(&number, &input(None, &[]))
            .await
            .expect("应写入成功");
        sqlx::query("UPDATE metadata SET field_sources = ?1 WHERE number = ?2")
            .bind("{not json")
            .bind(number.as_str())
            .execute(database.write())
            .await
            .expect("应写入原始文本");

        let result = repository.find(&number).await;

        match result {
            Err(error) => assert!(
                error.to_string().contains("field_sources"),
                "错误说明应指出是哪一列, 实际 {error}"
            ),
            Ok(_) => panic!("内容损坏时不应静默当作空来源表"),
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
            let snapshots: Vec<SourceSnapshotInput<'_>> = sources
                .iter()
                .map(|source| SourceSnapshotInput {
                    source_id: source,
                    payload: ScrapedMetadata::default(),
                })
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
                    ..MetadataFilter::default()
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
                    ..MetadataFilter::default()
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
    async fn locked_poster_survives_rescrape() {
        let (_dir, repository) = repository().await;
        let number = MediaNumber::parse("ABC-001").expect("应解析成功");
        let original = vec!["https://example.invalid/old.jpg".to_owned()];
        let row = repository
            .upsert(
                &number,
                &MetadataInput {
                    poster_urls: Some(&original),
                    ..MetadataInput::default()
                },
            )
            .await
            .expect("应写入成功");
        repository
            .set_lock(row.id, LockableField::PosterUrl, true)
            .await
            .expect("应加锁成功");

        // 海报是列表列, 锁定的判定与文本列走同一张字段表.
        let replacement = vec!["https://example.invalid/new.jpg".to_owned()];
        let applied = repository
            .apply_scraped(&ScrapeWrite {
                number: &number,
                input: MetadataInput {
                    poster_urls: Some(&replacement),
                    ..MetadataInput::default()
                },
                field_sources: BTreeMap::from([("poster_url".to_owned(), "dmm".to_owned())]),
            })
            .await
            .expect("应写入成功");

        assert_eq!(
            applied.row.poster_url(),
            Some("https://example.invalid/old.jpg"),
            "锁定的海报不应被覆盖"
        );
        assert_eq!(applied.kept, vec![LockableField::PosterUrl]);
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
    async fn upsert_respects_locks() {
        let (_dir, repository) = repository().await;
        let row = seeded_row(&repository, "人工标题").await;
        repository
            .set_lock(row.id, LockableField::Title, true)
            .await
            .expect("应加锁成功");
        let number = MediaNumber::parse("ABC-001").expect("应解析成功");

        let updated = repository
            .upsert(&number, &input(Some("覆盖标题"), &[]))
            .await
            .expect("应写入成功");

        assert_eq!(
            updated.title.as_deref(),
            Some("人工标题"),
            "创建入口同样不能绕过锁"
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
        let sources = row
            .field_sources
            .as_ref()
            .map(|json| json.0.to_map())
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
        let sources = applied
            .row
            .field_sources
            .as_ref()
            .map(|json| json.0.to_map())
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
        }

        let rows = repository
            .list(&MetadataFilter::default(), 10, 0)
            .await
            .expect("应列出成功");
        let numbers: Vec<&str> = rows.iter().map(|row| row.number.as_str()).collect();

        assert_eq!(numbers, ["ABC-003", "ABC-002", "ABC-001"]);
    }
}
