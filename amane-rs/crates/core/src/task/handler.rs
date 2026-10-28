use async_trait::async_trait;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use super::logger::TaskLogger;
use super::model::{TaskKind, TaskPayload, TaskSpec};

/// 处理失败的原因. 取消与失败是两种终态, 不合并.
#[derive(Debug, Error)]
pub enum TaskError {
    #[error("{0}")]
    Failed(String),
    #[error("任务已取消")]
    Cancelled,
}

/// 处理函数拿到的上下文.
pub struct TaskContext {
    pub task_id: i64,
    /// 已按类型还原的载荷; 处理函数不必再解析 JSON.
    pub payload: TaskPayload,
    pub logger: TaskLogger,
    /// 取消令牌; 处理函数在等待点检查它, 收到信号后尽快返回 [`TaskError::Cancelled`].
    pub cancel: CancellationToken,
}

/// 处理成功的结果.
#[derive(Debug, Default)]
pub struct TaskOutcome {
    /// 后继任务; 与本次完成在同一事务内入队.
    pub followups: Vec<TaskSpec>,
}

impl TaskOutcome {
    pub fn done() -> Self {
        Self::default()
    }

    pub fn with_followups(followups: Vec<TaskSpec>) -> Self {
        Self { followups }
    }
}

/// 一种任务类型的处理函数.
#[async_trait]
pub trait TaskHandler: Send + Sync {
    fn kind(&self) -> TaskKind;

    async fn run(&self, context: TaskContext) -> Result<TaskOutcome, TaskError>;
}
