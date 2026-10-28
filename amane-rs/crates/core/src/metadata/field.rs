//! 字段描述表.
//!
//! 一个可锁字段的展示、取值、清空、修订与聚合方式都在这张表里. 结构体字段声明与迁移之外,
//! 加字段不必再改按字段分派的地方.

use chrono::NaiveDate;

use crate::lockable::LockableField;

use super::{Metadata, MetadataInput, MetadataPatch, RepositoryError, ScrapedMetadata};

/// 一次写入里某个字段的取值.
///
/// 三种列各有自己的 SQL 类型; 绑定按取值分派, 不按字段分派.
pub enum FieldValue {
    /// 文本列; 空白按清空处理.
    Text(String),
    /// 日期列; `None` 写 NULL.
    Date(Option<NaiveDate>),
    /// 数值列; `None` 写 NULL.
    Number(Option<i32>),
}

impl FieldValue {
    fn text(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}

/// 一个可锁字段的描述.
pub struct FieldSpec {
    pub field: LockableField,
    /// 从条目读取展示值; 数值与日期在这里转成文本.
    pub read: fn(&Metadata) -> Option<String>,
    /// 从刮削输入读取本次提供的值; `None` 表示来源没有给出这一项.
    pub take: fn(&MetadataInput<'_>) -> Option<FieldValue>,
    /// 输入带有该字段的值时清空并返回 `true`; 用于剔除被锁定的字段.
    pub clear: fn(&mut MetadataInput<'_>) -> bool,
    /// 从手工修订读取本次提供的值; `None` 表示不改动.
    pub patch: fn(&MetadataPatch<'_>) -> Result<Option<FieldValue>, RepositoryError>,
    /// 聚合时把来源的非空值填入目标; 返回是否填入.
    pub merge: fn(&mut ScrapedMetadata, &ScrapedMetadata) -> bool,
}

const fn spec(
    field: LockableField,
    read: fn(&Metadata) -> Option<String>,
    take: fn(&MetadataInput<'_>) -> Option<FieldValue>,
    clear: fn(&mut MetadataInput<'_>) -> bool,
    patch: fn(&MetadataPatch<'_>) -> Result<Option<FieldValue>, RepositoryError>,
    merge: fn(&mut ScrapedMetadata, &ScrapedMetadata) -> bool,
) -> FieldSpec {
    FieldSpec {
        field,
        read,
        take,
        clear,
        patch,
        merge,
    }
}

/// 全部可锁字段, 顺序即详情页上的展示顺序.
///
/// 与 [`LockableField::ALL`] 的一致性由本模块的用例保证.
pub const FIELDS: [FieldSpec; 8] = [
    spec(
        LockableField::Title,
        read_title,
        take_title,
        clear_title,
        patch_title,
        merge_title,
    ),
    spec(
        LockableField::Plot,
        read_plot,
        take_plot,
        clear_plot,
        patch_plot,
        merge_plot,
    ),
    spec(
        LockableField::ReleaseDate,
        read_release_date,
        take_release_date,
        clear_release_date,
        patch_release_date,
        merge_release_date,
    ),
    spec(
        LockableField::RuntimeMinutes,
        read_runtime_minutes,
        take_runtime_minutes,
        clear_runtime_minutes,
        patch_runtime_minutes,
        merge_runtime_minutes,
    ),
    spec(
        LockableField::Maker,
        read_maker,
        take_maker,
        clear_maker,
        patch_maker,
        merge_maker,
    ),
    spec(
        LockableField::Label,
        read_label,
        take_label,
        clear_label,
        patch_label,
        merge_label,
    ),
    spec(
        LockableField::Series,
        read_series,
        take_series,
        clear_series,
        patch_series,
        merge_series,
    ),
    spec(
        LockableField::PosterUrl,
        read_poster_url,
        take_poster_url,
        clear_poster_url,
        patch_poster_url,
        merge_poster_url,
    ),
];

fn read_title(metadata: &Metadata) -> Option<String> {
    metadata.title.clone()
}

fn read_plot(metadata: &Metadata) -> Option<String> {
    metadata.plot.clone()
}

fn read_release_date(metadata: &Metadata) -> Option<String> {
    metadata.release_date.map(|date| date.to_string())
}

fn read_runtime_minutes(metadata: &Metadata) -> Option<String> {
    metadata.runtime_minutes.map(|minutes| minutes.to_string())
}

fn read_maker(metadata: &Metadata) -> Option<String> {
    metadata.maker.clone()
}

fn read_label(metadata: &Metadata) -> Option<String> {
    metadata.label.clone()
}

fn read_series(metadata: &Metadata) -> Option<String> {
    metadata.series.clone()
}

fn read_poster_url(metadata: &Metadata) -> Option<String> {
    metadata.poster_url.clone()
}

fn take_title(input: &MetadataInput<'_>) -> Option<FieldValue> {
    input.title.map(FieldValue::text)
}

fn take_plot(input: &MetadataInput<'_>) -> Option<FieldValue> {
    input.plot.map(FieldValue::text)
}

fn take_release_date(input: &MetadataInput<'_>) -> Option<FieldValue> {
    input.release_date.map(|date| FieldValue::Date(Some(date)))
}

fn take_runtime_minutes(input: &MetadataInput<'_>) -> Option<FieldValue> {
    input
        .runtime_minutes
        .map(|minutes| FieldValue::Number(Some(minutes)))
}

fn take_maker(input: &MetadataInput<'_>) -> Option<FieldValue> {
    input.maker.map(FieldValue::text)
}

fn take_label(input: &MetadataInput<'_>) -> Option<FieldValue> {
    input.label.map(FieldValue::text)
}

fn take_series(input: &MetadataInput<'_>) -> Option<FieldValue> {
    input.series.map(FieldValue::text)
}

fn take_poster_url(input: &MetadataInput<'_>) -> Option<FieldValue> {
    input.poster_url.map(FieldValue::text)
}

fn clear_title(input: &mut MetadataInput<'_>) -> bool {
    input.title.take().is_some()
}

fn clear_plot(input: &mut MetadataInput<'_>) -> bool {
    input.plot.take().is_some()
}

fn clear_release_date(input: &mut MetadataInput<'_>) -> bool {
    input.release_date.take().is_some()
}

fn clear_runtime_minutes(input: &mut MetadataInput<'_>) -> bool {
    input.runtime_minutes.take().is_some()
}

fn clear_maker(input: &mut MetadataInput<'_>) -> bool {
    input.maker.take().is_some()
}

fn clear_label(input: &mut MetadataInput<'_>) -> bool {
    input.label.take().is_some()
}

fn clear_series(input: &mut MetadataInput<'_>) -> bool {
    input.series.take().is_some()
}

fn clear_poster_url(input: &mut MetadataInput<'_>) -> bool {
    input.poster_url.take().is_some()
}

fn patch_title(patch: &MetadataPatch<'_>) -> Result<Option<FieldValue>, RepositoryError> {
    Ok(patch.title.map(FieldValue::text))
}

fn patch_plot(patch: &MetadataPatch<'_>) -> Result<Option<FieldValue>, RepositoryError> {
    Ok(patch.plot.map(FieldValue::text))
}

/// 上映日期是唯一需要解析的列: 空字符串表示写 NULL, 其余按 `%Y-%m-%d` 读.
fn patch_release_date(patch: &MetadataPatch<'_>) -> Result<Option<FieldValue>, RepositoryError> {
    match patch.release_date {
        None => Ok(None),
        Some(raw) if raw.trim().is_empty() => Ok(Some(FieldValue::Date(None))),
        Some(raw) => NaiveDate::parse_from_str(raw.trim(), "%Y-%m-%d")
            .map(|date| Some(FieldValue::Date(Some(date))))
            .map_err(|_| {
                RepositoryError::InvalidInput(format!("上映日期应形如 2021-02-18: {raw}"))
            }),
    }
}

fn patch_runtime_minutes(patch: &MetadataPatch<'_>) -> Result<Option<FieldValue>, RepositoryError> {
    Ok(patch
        .runtime_minutes
        .map(|minutes| FieldValue::Number(Some(minutes))))
}

fn patch_maker(patch: &MetadataPatch<'_>) -> Result<Option<FieldValue>, RepositoryError> {
    Ok(patch.maker.map(FieldValue::text))
}

fn patch_label(patch: &MetadataPatch<'_>) -> Result<Option<FieldValue>, RepositoryError> {
    Ok(patch.label.map(FieldValue::text))
}

fn patch_series(patch: &MetadataPatch<'_>) -> Result<Option<FieldValue>, RepositoryError> {
    Ok(patch.series.map(FieldValue::text))
}

/// 海报只能由刮削写入: 手工修订不提供这一项, 换图走重新刮削.
fn patch_poster_url(_patch: &MetadataPatch<'_>) -> Result<Option<FieldValue>, RepositoryError> {
    Ok(None)
}

/// 目标为空且来源有值时填入.
fn fill<T: Clone>(target: &mut Option<T>, source: &Option<T>) -> bool {
    if target.is_some() {
        return false;
    }
    match source {
        Some(value) => {
            *target = Some(value.clone());
            true
        }
        None => false,
    }
}

fn merge_title(target: &mut ScrapedMetadata, source: &ScrapedMetadata) -> bool {
    fill(&mut target.title, &source.title)
}

fn merge_plot(target: &mut ScrapedMetadata, source: &ScrapedMetadata) -> bool {
    fill(&mut target.plot, &source.plot)
}

fn merge_release_date(target: &mut ScrapedMetadata, source: &ScrapedMetadata) -> bool {
    fill(&mut target.release_date, &source.release_date)
}

fn merge_runtime_minutes(target: &mut ScrapedMetadata, source: &ScrapedMetadata) -> bool {
    fill(&mut target.runtime_minutes, &source.runtime_minutes)
}

fn merge_maker(target: &mut ScrapedMetadata, source: &ScrapedMetadata) -> bool {
    fill(&mut target.maker, &source.maker)
}

fn merge_label(target: &mut ScrapedMetadata, source: &ScrapedMetadata) -> bool {
    fill(&mut target.label, &source.label)
}

fn merge_series(target: &mut ScrapedMetadata, source: &ScrapedMetadata) -> bool {
    fill(&mut target.series, &source.series)
}

fn merge_poster_url(target: &mut ScrapedMetadata, source: &ScrapedMetadata) -> bool {
    fill(&mut target.poster_url, &source.poster_url)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{FIELDS, FieldValue};
    use crate::lockable::LockableField;
    use crate::metadata::{Metadata, MetadataInput, MetadataPatch, RepositoryError};

    #[test]
    fn table_covers_every_lockable_field() {
        let table: BTreeSet<LockableField> = FIELDS.iter().map(|spec| spec.field).collect();
        let declared: BTreeSet<LockableField> = LockableField::ALL.into_iter().collect();

        assert_eq!(table, declared, "表与枚举必须覆盖同一批字段");
        assert_eq!(FIELDS.len(), LockableField::ALL.len(), "不允许重复登记");
    }

    #[test]
    fn read_formats_date_and_minutes() {
        let metadata = Metadata {
            release_date: chrono::NaiveDate::from_ymd_opt(2021, 2, 18),
            runtime_minutes: Some(120),
            title: Some("标题".to_owned()),
            ..metadata_default()
        };

        assert_eq!(
            (FIELDS[2].read)(&metadata).as_deref(),
            Some("2021-02-18"),
            "日期按 yyyy-mm-dd 展示"
        );
        assert_eq!(
            (FIELDS[3].read)(&metadata).as_deref(),
            Some("120"),
            "时长按分钟数展示"
        );
        assert_eq!((FIELDS[0].read)(&metadata).as_deref(), Some("标题"));
        assert!((FIELDS[1].read)(&metadata).is_none(), "没有简介时为空");
    }

    #[test]
    fn clear_reports_whether_value_was_present() {
        let mut input = MetadataInput {
            title: Some("标题"),
            ..MetadataInput::default()
        };

        assert!((FIELDS[0].clear)(&mut input), "有值时报告已清空");
        assert!(input.title.is_none(), "值应被清空");
        assert!(!(FIELDS[0].clear)(&mut input), "再次清空不算有值");
        assert!(!(FIELDS[1].clear)(&mut input), "本来就没有的字段不算有值");
    }

    #[test]
    fn patch_reads_date_and_rejects_bad_format() {
        let patch = MetadataPatch {
            release_date: Some("2021-02-18"),
            ..MetadataPatch::default()
        };
        match (FIELDS[2].patch)(&patch).expect("应解析成功") {
            Some(FieldValue::Date(Some(date))) => {
                assert_eq!(date.to_string(), "2021-02-18");
            }
            _ => panic!("应给出日期取值"),
        }

        let empty = MetadataPatch {
            release_date: Some("  "),
            ..MetadataPatch::default()
        };
        assert!(
            matches!((FIELDS[2].patch)(&empty), Ok(Some(FieldValue::Date(None)))),
            "空白表示写 NULL"
        );

        let bad = MetadataPatch {
            release_date: Some("2021/02/18"),
            ..MetadataPatch::default()
        };
        match (FIELDS[2].patch)(&bad) {
            Err(RepositoryError::InvalidInput(message)) => {
                assert!(
                    message.contains("2021-02-18"),
                    "说明应给出正确格式: {message}"
                );
            }
            _ => panic!("格式不对应报错"),
        }
    }

    #[test]
    fn patch_skips_fields_the_request_did_not_offer() {
        let patch = MetadataPatch::default();

        for spec in FIELDS {
            assert!(
                (spec.patch)(&patch).expect("空修订不应报错").is_none(),
                "{} 在没有取值时不应产生写入",
                spec.field
            );
        }
    }

    fn metadata_default() -> Metadata {
        Metadata {
            id: 1,
            number: "ABC-001".to_owned(),
            title: None,
            plot: None,
            release_date: None,
            runtime_minutes: None,
            maker: None,
            label: None,
            series: None,
            poster_url: None,
            poster_resource_id: None,
            cover_url: None,
            cover_resource_id: None,
            field_sources: None,
            genres: None,
            note: None,
            content_type: crate::content_type::ContentType::Censored,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }
}
