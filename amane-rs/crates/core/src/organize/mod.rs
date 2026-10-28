//! 路径模板与整理预览、执行、清理: 渲染落点、判定冲突; 执行按预览落点移动文件, 清理先出清单再入回收目录.

pub mod aggregate;
pub mod cleanup;
pub mod execute;
pub mod model;
pub mod preview;
pub mod repository;
pub mod template;

pub use aggregate::{OrganizePreview, PreviewRequest, build_preview, number_from_path};
pub use cleanup::{
    CleanupError, CleanupItem, RestoreError, TrashItem, build_cleanup_manifest, execute_cleanup,
    list_trash, restore_from_trash, trash_total_size,
};
pub use execute::{
    ConflictAction, DEFAULT_SUBTITLE_EXTENSIONS, ExecuteError, ExecuteRequest, ItemOutcome,
    PlanRegistration, TRASH_DIRNAME, execute_batch, execute_item, move_to_trash, rename_candidate,
};
pub use model::OrganizePlan;
pub use preview::{
    ConflictKind, PreviewItem, TargetContext, classify_conflict, render_target, variables_for,
};
pub use repository::{OrganizeError, OrganizeRepository};
pub use template::{PLACEHOLDERS, Template, TemplateError};
