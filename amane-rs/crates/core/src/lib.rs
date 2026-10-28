//! Amane 的业务内核: 不依赖传输层, 不持有全局可变状态.
//!
//! 上层 (`amane-server` / `amane-desktop`) 只通过这里导出的类型与仓储访问持久化,
//! 传输格式与协议细节留在各自 crate.

pub mod config;
pub mod content_type;
pub mod db;
pub mod metadata;
pub mod number;
pub mod resource;
pub mod task;

pub use content_type::ContentType;
pub use db::{Database, DbError};
pub use metadata::{Metadata, MetadataFilter, MetadataInput, MetadataRepository, RepositoryError};
pub use number::{MediaNumber, NumberError};
pub use resource::{Resource, ResourceError, ResourceRepository};
pub use task::{
    Task, TaskContext, TaskError, TaskFilter, TaskHandler, TaskKind, TaskLogLine, TaskLogger,
    TaskOutcome, TaskRepository, TaskRepositoryError, TaskService, TaskServiceError, TaskSpec,
    TaskStatus,
};
