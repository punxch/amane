use std::fmt;

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// 任务类型. 每种类型对应一个处理函数.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, sqlx::Type,
)]
#[serde(rename_all = "snake_case")]
#[sqlx(rename_all = "snake_case")]
pub enum TaskKind {
    Scrape,
    /// 取回条目的海报与封面; 由刮削派生.
    Assets,
    /// 扫描一个媒体库目录, 归类文件并记录增量.
    Scan,
    Organize,
    Refresh,
    Cleanup,
}

impl TaskKind {
    /// 全部取值. `as_str` 是唯一的字符串表, 解析从这里反查.
    pub const ALL: [Self; 6] = [
        Self::Scrape,
        Self::Assets,
        Self::Scan,
        Self::Organize,
        Self::Refresh,
        Self::Cleanup,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Scrape => "scrape",
            Self::Assets => "assets",
            Self::Scan => "scan",
            Self::Organize => "organize",
            Self::Refresh => "refresh",
            Self::Cleanup => "cleanup",
        }
    }

    /// 由外部字符串还原; 未知取值返回 `None`, 由调用方决定如何报错.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == value)
    }
}

impl fmt::Display for TaskKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// 任务状态. 只有 排队 与 运行 不是终态.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, sqlx::Type,
)]
#[serde(rename_all = "snake_case")]
#[sqlx(rename_all = "snake_case")]
pub enum TaskStatus {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
}

impl TaskStatus {
    /// 全部取值. `as_str` 是唯一的字符串表, 解析从这里反查.
    pub const ALL: [Self; 5] = [
        Self::Queued,
        Self::Running,
        Self::Done,
        Self::Failed,
        Self::Cancelled,
    ];

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|status| status.as_str() == value)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    /// 只有失败与已取消允许重试: 完成任务重试会重复副作用.
    pub const fn is_retryable(self) -> bool {
        matches!(self, Self::Failed | Self::Cancelled)
    }

    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Cancelled)
    }
}

/// 入队请求. 载荷由各类型自行解释, 队列不感知其结构.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSpec {
    pub kind: TaskKind,
    pub payload: serde_json::Value,
    pub priority: i32,
    /// 定时登记的标识; 手工提交为 `None`.
    pub schedule_id: Option<i64>,
}

impl TaskSpec {
    pub fn new(kind: TaskKind, payload: serde_json::Value) -> Self {
        Self {
            kind,
            payload,
            priority: 0,
            schedule_id: None,
        }
    }
}

/// 一条任务记录.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Task {
    pub id: i64,
    pub kind: TaskKind,
    pub payload: String,
    pub status: TaskStatus,
    pub priority: i32,
    pub attempts: i32,
    pub error: Option<String>,
    pub retry_of: Option<i64>,
    /// 由哪条定时任务派生; 手工提交为 `None`.
    pub schedule_id: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}
