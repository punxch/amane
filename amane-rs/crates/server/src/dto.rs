use amane_core::{Metadata, Task, TaskLogLine};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// 条目的传输形态.
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
    /// 演员名单; 列表端点不返回, 只有详情端点填充.
    pub actors: Vec<String>,
    /// 条目类型, 取值见 `ContentType`.
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
            actors: Vec::new(),
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
    /// 单页条数, 默认 50, 上限 200.
    pub limit: Option<u32>,
    pub offset: Option<u32>,
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
