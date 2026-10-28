//! 路径模板与整理预览、执行: 渲染落点、判定冲突; 执行按预览落点移动文件.

pub mod aggregate;
pub mod execute;
pub mod model;
pub mod preview;
pub mod repository;
pub mod template;

pub use aggregate::{OrganizePreview, PreviewRequest, build_preview, number_from_path};
pub use execute::{
    ConflictAction, DEFAULT_SUBTITLE_EXTENSIONS, ExecuteError, ExecuteRequest, ItemOutcome,
    OrganizePlanStub, TRASH_DIRNAME, execute_batch, execute_item, rename_candidate,
};
pub use model::OrganizePlan;
pub use preview::{
    ConflictKind, PreviewItem, TargetContext, classify_conflict, render_target, variables_for,
};
pub use repository::{OrganizeError, OrganizeRepository};
pub use template::{PLACEHOLDERS, Template, TemplateError};
