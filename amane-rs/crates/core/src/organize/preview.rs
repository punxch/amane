//! 整理预览: 只读地给出每个文件的落点与冲突, 不碰磁盘.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::template::Template;
use crate::MediaNumber;
use crate::metadata::Metadata;

/// 三类冲突. 一个文件只标一种, 按严重程度取最先命中的一种.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictKind {
    /// 目标路径已在磁盘上存在.
    Exists,
    /// 目标路径已被另一个番号的整理记录占用.
    NumberMismatch,
    /// 源与目标相同, 文件已在目标位置.
    SamePath,
}

impl ConflictKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Exists => "exists",
            Self::NumberMismatch => "number_mismatch",
            Self::SamePath => "same_path",
        }
    }
}

/// 预览的一项.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PreviewItem {
    /// 源文件, 相对库根的 POSIX 路径.
    pub source_path: String,
    /// 渲染出的目标路径, 相对库根的 POSIX 路径.
    pub target_path: String,
    /// 由文件名提取并归一的番号.
    pub number: String,
    pub conflict: Option<ConflictKind>,
}

/// 渲染目标所需的全部输入. 条目可以缺席: 缺席时只有番号参与渲染.
#[derive(Debug, Clone)]
pub struct TargetContext<'a> {
    pub number: &'a str,
    pub entry: Option<&'a Metadata>,
    /// 不带点的小写扩展名, 如 `mp4`.
    pub ext: &'a str,
    /// 源文件相对库根的路径, 用于 `raw_dir` / `raw_name`.
    pub source_rel: &'a str,
}

/// 由条目与源文件组装模板变量.
///
/// 变量值已经过 Windows 非法字符清洗; 缺席字段输出空串, 由可选组折叠.
pub fn variables_for(context: &TargetContext) -> BTreeMap<String, String> {
    let mut variables = BTreeMap::new();
    let entry = context.entry;
    variables.insert("number".to_owned(), context.number.to_owned());
    variables.insert(
        "prefix".to_owned(),
        MediaNumber::parse(context.number)
            .map(|number| number.prefix().to_owned())
            .unwrap_or_default(),
    );
    variables.insert(
        "suffix".to_owned(),
        context
            .number
            .split_once('-')
            .map(|(_, tail)| tail.to_owned())
            .unwrap_or_default(),
    );
    variables.insert(
        "title".to_owned(),
        entry
            .and_then(|row| row.title.as_deref())
            .map(sanitize_text)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| context.number.to_owned()),
    );
    for (name, value) in [
        ("maker", entry.and_then(|row| row.maker.as_deref())),
        ("label", entry.and_then(|row| row.label.as_deref())),
        ("series", entry.and_then(|row| row.series.as_deref())),
    ] {
        variables.insert(
            name.to_owned(),
            value.map(sanitize_text).unwrap_or_default(),
        );
    }
    let year = entry
        .and_then(|row| row.release_date)
        .map(|date| date.format("%Y").to_string());
    variables.insert("year".to_owned(), year.unwrap_or_default());
    variables.insert(
        "release".to_owned(),
        entry
            .and_then(|row| row.release_date)
            .map(|date| date.to_string())
            .unwrap_or_default(),
    );
    variables.insert(
        "content_type".to_owned(),
        entry
            .map(|row| row.content_type.as_str().to_owned())
            .unwrap_or_default(),
    );
    variables.insert("ext".to_owned(), context.ext.to_owned());
    variables.insert("raw_dir".to_owned(), raw_dir_of(context.source_rel));
    variables.insert(
        "raw_name".to_owned(),
        raw_name_of(context.source_rel).to_owned(),
    );
    variables
}

fn sanitize_text(value: &str) -> String {
    super::template::sanitize_field(value)
}

/// 源路径的父目录名; 顶层文件没有父目录, 输出空串.
fn raw_dir_of(source_rel: &str) -> String {
    let trimmed = source_rel.trim_end_matches('/');
    match trimmed.rsplit_once('/') {
        Some((parent, _)) => parent.to_owned(),
        None => String::new(),
    }
}

/// 源路径的文件名主干, 不含扩展名.
fn raw_name_of(source_rel: &str) -> String {
    let file = source_rel.rsplit('/').next().unwrap_or(source_rel);
    match file.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem.to_owned(),
        _ => file.to_owned(),
    }
}

/// 渲染一个文件的目标路径.
pub fn render_target(template: &Template, context: &TargetContext) -> String {
    template.render(&variables_for(context))
}

/// 判定冲突. 判定顺序: 源与目标相同 → 登记占用 → 磁盘已存在.
///
/// 「同名不同番号」依赖整理登记; 空登记表时该类冲突不会出现, 属于预期.
pub fn classify_conflict(
    source_rel: &str,
    target_rel: &str,
    root: &Path,
    source_number: &str,
    existing_number: Option<&str>,
) -> Option<ConflictKind> {
    if source_rel == target_rel {
        return Some(ConflictKind::SamePath);
    }
    if let Some(number) = existing_number
        && number != source_number
    {
        return Some(ConflictKind::NumberMismatch);
    }
    // symlink_metadata 不跟随链接: 指向别处的链接也算占用目标位置.
    if root.join(target_rel).symlink_metadata().is_ok() {
        return Some(ConflictKind::Exists);
    }
    None
}
