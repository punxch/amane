//! 任务: 排队、执行、取消与重试.
//!
//! 队列落在数据库, 进程重启后未完成的任务重新被领取; 处理者按并发上限并发出队.

mod handler;
mod logger;
mod model;
mod repository;
mod service;

pub use handler::{TaskContext, TaskError, TaskHandler, TaskOutcome};
pub use logger::{TaskLogLine, TaskLogger};
pub use model::{Task, TaskKind, TaskSpec, TaskStatus};
pub use repository::{TaskFilter, TaskRepository, TaskRepositoryError};
pub use service::{TaskService, TaskServiceError};
