//! Amane 的业务内核: 不依赖传输层, 不持有全局可变状态.
//!
//! 上层 (`amane-server` / `amane-desktop`) 只通过这里导出的类型与仓储访问持久化,
//! 传输格式与协议细节留在各自 crate.

pub mod config;
pub mod content_type;
pub mod db;
pub mod library;
pub mod lockable;
pub mod metadata;
pub mod number;
pub mod organize;
pub mod resource;
pub mod schedule;
pub mod task;

pub use content_type::ContentType;
pub use db::{Database, DbError};
pub use library::{
    KindCounts, LibraryError, LibraryFile, LibraryRepository, LibraryRoot, ScanDiff, ScanError,
    ScanKind, ScanRules, UnwantedRule, scan_directory,
};
pub use lockable::LockableField;
pub use metadata::{
    FieldSources, MANUAL_SOURCE, Metadata, MetadataFilter, MetadataInput, MetadataPatch,
    MetadataRemoval, MetadataRepository, RepositoryError, ResourceFiles, ScrapeApplied,
    ScrapeWrite, ScrapedMetadata, SourceSnapshot, SourceSnapshotInput,
};
pub use number::{MediaNumber, NumberError};
pub use organize::{
    CleanupItem, ConflictAction, ConflictKind, OrganizeError, OrganizePlan, OrganizePreview,
    OrganizeRepository, PLACEHOLDERS, PreviewAssembler, PreviewError, PreviewItem, PreviewRequest,
    Template, TemplateError, TrashItem, build_cleanup_manifest, build_preview, execute_batch,
    execute_cleanup, list_trash, number_from_path, restore_from_trash, trash_total_size,
};
pub use resource::{Resource, ResourceError, ResourceRepository};
pub use schedule::{
    Pattern, Schedule, ScheduleError, SchedulePatch, ScheduleRepository, ScheduleSpec, Scheduler,
};
pub use task::{
    Task, TaskContext, TaskError, TaskFilter, TaskHandler, TaskKind, TaskLogLine, TaskLogger,
    TaskOutcome, TaskRepository, TaskRepositoryError, TaskService, TaskServiceError, TaskSpec,
    TaskStatus,
};
