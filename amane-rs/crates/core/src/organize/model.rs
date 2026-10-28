//! 整理预览与登记的数据模型.

use chrono::{DateTime, Utc};
use serde::Serialize;

/// 整理登记: 一次执行写入一行, 预览据此判「同名不同番号」.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow, Serialize)]
pub struct OrganizePlan {
    pub id: i64,
    pub root_id: i64,
    /// 目标视频路径, 相对库根的 POSIX 路径.
    pub path: String,
    pub number: String,
    /// 执行时的源路径; 清理与回收切片据此回溯.
    pub source_path: String,
    pub created_at: DateTime<Utc>,
}
