//! 路径模板与整理预览: 渲染落点、判定冲突; 执行与清理属于后续切片.

pub mod aggregate;
pub mod model;
pub mod preview;
pub mod repository;
pub mod template;

pub use aggregate::{OrganizePreview, PreviewRequest, build_preview, number_from_path};
pub use model::OrganizePlan;
pub use preview::{
    ConflictKind, PreviewItem, TargetContext, classify_conflict, render_target, variables_for,
};
pub use repository::{OrganizeError, OrganizeRepository};
pub use template::{PLACEHOLDERS, Template, TemplateError};
