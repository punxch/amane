use amane_core::{Metadata, Task, TaskLogLine};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// 番号条目的传输形态.
///
/// 与 `amane_core::Metadata` 分开: 内核不感知传输层, 字段增删不会波及领域类型.
#[derive(Debug, Serialize, ToSchema)]
pub struct MetadataDto {
    pub id: i64,
    pub number: String,
    pub title: Option<String>,
    pub plot: Option<String>,
    /// 上映日期, 形如 `2021-02-18`.
    pub release_date: Option<String>,
    /// 时长, 单位分钟.
    pub runtime_minutes: Option<i32>,
    pub maker: Option<String>,
    pub label: Option<String>,
    pub series: Option<String>,
    pub poster_url: Option<String>,
    /// 海报在服务端的地址; 为 `None` 时表示尚未存到本地.
    pub poster_local_url: Option<String>,
    /// 演员名单; 列表端点不返回, 只有详情端点填充.
    pub actors: Vec<String>,
    /// 本次刮削命中的来源标识; 只有详情端点填充.
    pub sources: Vec<String>,
    /// 字段名 → 贡献该字段的来源标识.
    pub field_sources: std::collections::BTreeMap<String, String>,
    /// 类型, 取值见 `ContentType`.
    pub content_type: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<Metadata> for MetadataDto {
    fn from(value: Metadata) -> Self {
        Self {
            id: value.id,
            number: value.number,
            title: value.title,
            plot: value.plot,
            release_date: value.release_date.map(|date| date.to_string()),
            runtime_minutes: value.runtime_minutes,
            maker: value.maker,
            label: value.label,
            series: value.series,
            poster_url: value.poster_url,
            poster_local_url: value
                .poster_resource_id
                .map(|id| format!("/api/resources/{id}")),
            actors: Vec::new(),
            sources: Vec::new(),
            field_sources: value
                .field_sources
                .as_deref()
                .and_then(|raw| serde_json::from_str(raw).ok())
                .unwrap_or_default(),
            content_type: value.content_type.as_str().to_owned(),
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MetadataPage {
    /// 命中总数, 不受分页影响.
    pub total: i64,
    pub items: Vec<MetadataDto>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateMetadataRequest {
    /// 番号; 允许带分隔符或连写, 服务端归一后写入.
    pub number: String,
    pub title: Option<String>,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListQuery {
    /// 单页条数, 默认 50, 上限 200; 小于 1 按 1 处理.
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    /// 类型: censored / uncensored / chinese / western / fc2 / amateur / hentai.
    pub content_type: Option<String>,
    /// 来源标识; 只列出该来源贡献过内容的番号条目.
    pub source: Option<String>,
    /// 关键字, 匹配番号与标题, 忽略大小写.
    pub q: Option<String>,
}

/// 筛选可选项. 取值域由服务端给出, 界面不自行维护枚举.
#[derive(Debug, Serialize, ToSchema)]
pub struct FilterOptions {
    pub content_types: Vec<ContentTypeOption>,
    pub sources: Vec<SourceOption>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ContentTypeOption {
    pub value: String,
    pub label: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SourceOption {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ExtractQuery {
    /// 待解析的自由文本, 通常是文件名或完整路径.
    pub text: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ExtractedNumber {
    pub number: String,
    pub content_type: String,
    pub prefix: String,
}

/// 任务的传输形态.
#[derive(Debug, Serialize, ToSchema)]
pub struct TaskDto {
    pub id: i64,
    /// 任务类型, 取值见 `TaskKind`.
    pub kind: String,
    /// 任务状态, 取值见 `TaskStatus`.
    pub status: String,
    pub payload: serde_json::Value,
    pub priority: i32,
    pub attempts: i32,
    pub error: Option<String>,
    /// 重试来源任务的标识.
    pub retry_of: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

impl From<Task> for TaskDto {
    fn from(value: Task) -> Self {
        let payload = serde_json::from_str(&value.payload).unwrap_or(serde_json::Value::Null);
        Self {
            id: value.id,
            kind: value.kind.to_string(),
            status: value.status.as_str().to_owned(),
            payload,
            priority: value.priority,
            attempts: value.attempts,
            error: value.error,
            retry_of: value.retry_of,
            created_at: value.created_at,
            started_at: value.started_at,
            finished_at: value.finished_at,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TaskPage {
    pub total: i64,
    pub items: Vec<TaskDto>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateTaskRequest {
    /// 任务类型: scrape / organize / refresh / cleanup. 目前只有 scrape 注册了处理函数.
    pub kind: String,
    /// 任务载荷, 结构由类型决定.
    pub payload: serde_json::Value,
    #[serde(default)]
    pub priority: i32,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct TaskListQuery {
    /// 按状态过滤: queued / running / done / failed / cancelled.
    pub status: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TaskLogEntry {
    pub at: DateTime<Utc>,
    pub level: String,
    pub message: String,
}

impl From<TaskLogLine> for TaskLogEntry {
    fn from(value: TaskLogLine) -> Self {
        Self {
            at: value.at,
            level: value.level,
            message: value.message,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TaskLogPage {
    pub items: Vec<TaskLogEntry>,
}
