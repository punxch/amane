use std::fmt;

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

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

/// 任务载荷. 每个变体自带任务类型, 因此类型与载荷不可能对不上.
///
/// 落库时只存载荷本体, 类型另存 `task.kind` 一列; 读回时按那一列还原变体.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskPayload {
    Scrape {
        number: String,
    },
    Assets {
        number: String,
    },
    Scan {
        library_id: i64,
    },
    Organize {
        library_id: i64,
        choices: BTreeMap<String, String>,
    },
    Refresh,
    Cleanup {
        library_id: i64,
    },
}

impl TaskPayload {
    pub const fn kind(&self) -> TaskKind {
        match self {
            Self::Scrape { .. } => TaskKind::Scrape,
            Self::Assets { .. } => TaskKind::Assets,
            Self::Scan { .. } => TaskKind::Scan,
            Self::Organize { .. } => TaskKind::Organize,
            Self::Refresh => TaskKind::Refresh,
            Self::Cleanup { .. } => TaskKind::Cleanup,
        }
    }

    /// 落库形态: 只有载荷本体, 不含类型.
    pub fn body(&self) -> serde_json::Value {
        match self {
            Self::Scrape { number } | Self::Assets { number } => {
                serde_json::json!({ "number": number })
            }
            Self::Scan { library_id } | Self::Cleanup { library_id } => {
                serde_json::json!({ "library_id": library_id })
            }
            Self::Organize {
                library_id,
                choices,
            } => serde_json::json!({ "library_id": library_id, "choices": choices }),
            Self::Refresh => serde_json::json!({}),
        }
    }

    /// 按类型还原载荷.
    pub fn parse(kind: TaskKind, body: serde_json::Value) -> Result<Self, PayloadError> {
        let payload = match kind {
            TaskKind::Scrape => Self::Scrape {
                number: take::<NumberRef>(kind, body)?.number,
            },
            TaskKind::Assets => Self::Assets {
                number: take::<NumberRef>(kind, body)?.number,
            },
            TaskKind::Scan => Self::Scan {
                library_id: take::<LibraryRef>(kind, body)?.library_id,
            },
            TaskKind::Cleanup => Self::Cleanup {
                library_id: take::<LibraryRef>(kind, body)?.library_id,
            },
            TaskKind::Organize => {
                let body = take::<OrganizeBody>(kind, body)?;
                Self::Organize {
                    library_id: body.library_id,
                    choices: body.choices,
                }
            }
            TaskKind::Refresh => Self::Refresh,
        };

        Ok(payload)
    }
}

#[derive(Debug, Error)]
#[error("任务载荷不合法 ({kind}): {message}")]
pub struct PayloadError {
    pub kind: TaskKind,
    pub message: String,
}

fn take<T: serde::de::DeserializeOwned>(
    kind: TaskKind,
    body: serde_json::Value,
) -> Result<T, PayloadError> {
    serde_json::from_value(body).map_err(|error| PayloadError {
        kind,
        message: error.to_string(),
    })
}

#[derive(Deserialize)]
struct NumberRef {
    number: String,
}

#[derive(Deserialize)]
struct LibraryRef {
    library_id: i64,
}

#[derive(Deserialize)]
struct OrganizeBody {
    library_id: i64,
    #[serde(default)]
    choices: BTreeMap<String, String>,
}

/// 入队请求.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSpec {
    pub payload: TaskPayload,
    pub priority: i32,
    /// 定时登记的标识; 手工提交为 `None`.
    pub schedule_id: Option<i64>,
}

impl TaskSpec {
    pub fn new(payload: TaskPayload) -> Self {
        Self {
            payload,
            priority: 0,
            schedule_id: None,
        }
    }

    pub const fn kind(&self) -> TaskKind {
        self.payload.kind()
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

#[cfg(test)]
mod tests {
    use super::{TaskKind, TaskPayload};

    #[test]
    fn body_round_trips_through_parse() {
        let payload = TaskPayload::Organize {
            library_id: 3,
            choices: [("ABC-123.mp4".to_owned(), "rename".to_owned())].into(),
        };

        let restored =
            TaskPayload::parse(payload.kind(), payload.body()).expect("应还原成同一份载荷");

        assert_eq!(restored, payload);
    }

    #[test]
    fn parse_rejects_body_missing_required_field() {
        let result = TaskPayload::parse(TaskKind::Scan, serde_json::json!({ "id": 1 }));

        match result {
            Err(error) => {
                let message = error.to_string();
                assert!(message.contains("scan"), "说明应指出任务类型: {message}");
                assert!(
                    message.contains("library_id"),
                    "说明应指出缺的字段: {message}"
                );
            }
            Ok(_) => panic!("缺字段的载荷不应被接受"),
        }
    }
}
