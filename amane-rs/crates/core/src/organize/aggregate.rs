//! 预览装配: 对一组媒体文件渲染目标路径并标注冲突; 不写盘、不建目录, 重复执行结果一致.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;

use super::preview::{PreviewItem, TargetContext, classify_conflict, render_target};
use super::template::Template;
use crate::MediaNumber;

/// 预览的输入: 一个已配对番号的媒体文件.
#[derive(Debug, Clone)]
pub struct PreviewRequest<'a> {
    /// 相对库根的 POSIX 路径.
    pub source_rel: &'a str,
    pub number: &'a str,
    /// 番号对应的条目; 缺席时只有番号参与渲染.
    pub entry: Option<&'a crate::Metadata>,
}

/// 预览结果.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct OrganizePreview {
    pub items: Vec<PreviewItem>,
    /// 提取不出番号而无法参与整理的媒体文件数.
    pub unmatched: i64,
    /// 索引里有但磁盘上已不存在的源文件数 (整理过或被外部移动).
    pub stale: i64,
}

/// 组装预览. `root` 只用于探测磁盘占用; 除探测外不触碰文件系统.
///
/// 目标路径使用正斜杠; 交给执行切片时同一份文本即目标路径, 保证「执行与预览一致」.
pub fn build_preview(
    template: &Template,
    root: &Path,
    requests: &[PreviewRequest],
    plans: &BTreeMap<String, String>,
) -> OrganizePreview {
    let mut items = Vec::with_capacity(requests.len());
    let mut unmatched = 0i64;
    let mut stale = 0i64;

    for request in requests {
        // 索引与磁盘脱节: 源已不存在的条目无法整理, 单独计数不误报冲突.
        if root.join(request.source_rel).symlink_metadata().is_err() {
            stale += 1;
            continue;
        }
        let ext = extension_of(request.source_rel);
        let context = TargetContext {
            number: request.number,
            entry: request.entry,
            ext: &ext,
            source_rel: request.source_rel,
        };
        let target = render_target(template, &context);
        if target.is_empty() {
            // 模板折叠到只剩空串 (如只有空字段的目录段), 无法整理, 归入未配对.
            unmatched += 1;
            continue;
        }

        let conflict = classify_conflict(
            request.source_rel,
            &target,
            root,
            request.number,
            plans.get(&target).map(String::as_str),
        );
        items.push(PreviewItem {
            source_path: request.source_rel.to_owned(),
            target_path: target,
            number: request.number.to_owned(),
            conflict,
        });
    }

    OrganizePreview {
        items,
        unmatched,
        stale,
    }
}

/// 从文件名提取番号; 提取不出的文件无法参与整理.
pub fn number_from_path(source_rel: &str) -> Option<MediaNumber> {
    let file = source_rel.rsplit('/').next().unwrap_or(source_rel);
    MediaNumber::extract(file)
}

fn extension_of(source_rel: &str) -> String {
    let file = source_rel.rsplit('/').next().unwrap_or(source_rel);
    file.rsplit_once('.')
        .map(|(_, ext)| ext.to_lowercase())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;

    use super::super::template::Template;
    use super::{PreviewRequest, number_from_path};
    use crate::MediaNumber;
    use crate::MetadataInput;
    use crate::MetadataRepository;
    use crate::library::model::LibraryFile;
    use crate::library::repository::LibraryRepository;

    fn request<'a>(
        source_rel: &'a str,
        number: &'a str,
        entry: Option<&'a crate::Metadata>,
    ) -> PreviewRequest<'a> {
        PreviewRequest {
            source_rel,
            number,
            entry,
        }
    }

    #[tokio::test]
    async fn renders_targets_from_entries() {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let database = crate::Database::open(&dir.path().join("amane.db"))
            .await
            .expect("应打开数据库");
        let metadata = MetadataRepository::new(database);
        let number = MediaNumber::parse("ABC-123").expect("应解析成功");
        let row = metadata
            .upsert(
                &number,
                &MetadataInput {
                    title: Some("标题"),
                    maker: Some("SOD"),
                    ..MetadataInput::default()
                },
            )
            .await
            .expect("应写入条目");

        let template =
            Template::parse("{maker}/{number}/{number}[-{title}].{ext}").expect("应解析成功");
        std::fs::create_dir_all(dir.path().join("downloads/a")).expect("应创建目录");
        std::fs::write(dir.path().join("downloads/a/ABC-123.mp4"), b"v").expect("应写入源");
        let preview = super::build_preview(
            &template,
            dir.path(),
            &[request("downloads/a/ABC-123.mp4", "ABC-123", Some(&row))],
            &BTreeMap::new(),
        );

        assert_eq!(preview.unmatched, 0);
        assert_eq!(preview.items.len(), 1);
        assert_eq!(preview.items[0].target_path, "SOD/ABC-123/ABC-123-标题.mp4");
        assert_eq!(preview.items[0].conflict, None);
    }

    #[tokio::test]
    async fn entries_without_title_fall_back_to_number() {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let database = crate::Database::open(&dir.path().join("amane.db"))
            .await
            .expect("应打开数据库");
        let metadata = MetadataRepository::new(database);
        let number = MediaNumber::parse("ABC-123").expect("应解析成功");
        let row = metadata
            .upsert(&number, &MetadataInput::default())
            .await
            .expect("应写入条目");

        let template = Template::parse("{number}/{title}.{ext}").expect("应解析成功");
        std::fs::create_dir_all(dir.path().join("a")).expect("应创建目录");
        std::fs::write(dir.path().join("a/ABC-123.mp4"), b"v").expect("应写入源");
        let preview = super::build_preview(
            &template,
            dir.path(),
            &[request("a/ABC-123.mp4", "ABC-123", Some(&row))],
            &BTreeMap::new(),
        );

        assert_eq!(preview.items[0].target_path, "ABC-123/ABC-123.mp4");
    }

    #[tokio::test]
    async fn same_path_conflict_detected() {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let template = Template::parse("{number}.{ext}").expect("应解析成功");
        // 源文件必须真实存在, 否则计入 stale 而不是冲突.
        fs::write(dir.path().join("ABC-123.mp4"), b"v").expect("应写入源");

        let preview = super::build_preview(
            &template,
            dir.path(),
            &[request("ABC-123.mp4", "ABC-123", None)],
            &BTreeMap::new(),
        );

        assert_eq!(
            preview.items[0].conflict,
            Some(super::super::preview::ConflictKind::SamePath)
        );
    }

    #[tokio::test]
    async fn exists_conflict_detected_from_disk() {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        // 磁盘上已有同路径文件, 但番号不同 (源是另一个番号).
        fs::create_dir_all(dir.path().join("XYZ-999")).expect("应创建目录");
        fs::write(dir.path().join("XYZ-999/XYZ-999.mp4"), b"x").expect("应写入文件");
        fs::create_dir_all(dir.path().join("downloads")).expect("应创建目录");
        fs::write(dir.path().join("downloads/XYZ-999.mp4"), b"v").expect("应写入源");

        let template = Template::parse("{number}/{number}.{ext}").expect("应解析成功");
        let preview = super::build_preview(
            &template,
            dir.path(),
            &[request("downloads/XYZ-999.mp4", "XYZ-999", None)],
            &BTreeMap::new(),
        );

        assert_eq!(
            preview.items[0].conflict,
            Some(super::super::preview::ConflictKind::Exists)
        );
    }

    #[tokio::test]
    async fn number_mismatch_conflict_from_plan() {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let template = Template::parse("{number}/{number}.{ext}").expect("应解析成功");
        fs::create_dir_all(dir.path().join("downloads")).expect("应创建目录");
        fs::write(dir.path().join("downloads/ABC-123.mp4"), b"v").expect("应写入源");

        // 目标路径被另一个番号的整理记录占用.
        let mut plans = BTreeMap::new();
        plans.insert("ABC-123/ABC-123.mp4".to_owned(), "XYZ-999".to_owned());

        let preview = super::build_preview(
            &template,
            dir.path(),
            &[request("downloads/ABC-123.mp4", "ABC-123", None)],
            &plans,
        );

        assert_eq!(
            preview.items[0].conflict,
            Some(super::super::preview::ConflictKind::NumberMismatch)
        );
    }

    #[tokio::test]
    async fn unmatched_files_are_counted_not_listed() {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        // number 与 maker 都空: 模板折叠到只剩分隔符, 渲染结果为空串, 无法整理.
        let template = Template::parse("{maker}/{number}").expect("应解析成功");
        std::fs::create_dir_all(dir.path().join("downloads")).expect("应创建目录");
        std::fs::write(dir.path().join("downloads/毫无番号.mp4"), b"v").expect("应写入源");

        let preview = super::build_preview(
            &template,
            dir.path(),
            &[request("downloads/毫无番号.mp4", "", None)],
            &BTreeMap::new(),
        );

        assert!(preview.items.is_empty());
        assert_eq!(preview.unmatched, 1);
    }

    #[tokio::test]
    async fn stale_entries_are_counted_not_conflicts() {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let template = Template::parse("{number}/{number}.{ext}").expect("应解析成功");

        // 索引里有, 磁盘上没有: 整理执行后的典型状态.
        let preview = super::build_preview(
            &template,
            dir.path(),
            &[request("ABC-123/ABC-123.mp4", "ABC-123", None)],
            &BTreeMap::new(),
        );

        assert!(preview.items.is_empty(), "过期条目不应误报冲突");
        assert_eq!(preview.stale, 1);
    }

    #[test]
    fn numbers_come_from_file_names() {
        assert_eq!(
            number_from_path("downloads/ABC-123.mp4")
                .expect("应提取成功")
                .as_str(),
            "ABC-123"
        );
        assert!(number_from_path("downloads/没有番号.mp4").is_none());
    }

    /// 扫描索引与预览的衔接: media 文件按文件名配对番号后进入预览.
    #[tokio::test]
    async fn integrates_with_scan_index() {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let database = crate::Database::open(&dir.path().join("amane.db"))
            .await
            .expect("应打开数据库");
        let libraries = LibraryRepository::new(database);
        let root = libraries.insert(dir.path()).await.expect("应登记成功");
        libraries
            .replace_files(
                root.id,
                &[
                    LibraryFile::new("downloads/ABC-123.mp4", crate::ScanKind::Media),
                    LibraryFile::new("downloads/plain.mp4", crate::ScanKind::Media),
                ],
                &[],
            )
            .await
            .expect("应写入索引");

        let (_, media) = libraries
            .files(root.id, Some(crate::ScanKind::Media), 100, 0)
            .await
            .expect("应查询成功");
        // 先配对出 (源路径, 番号), 再借引用构造请求; 番号字符串要活得比请求久.
        let paired: Vec<(String, String)> = media
            .iter()
            .filter_map(|file| {
                let number = super::number_from_path(&file.path)?;
                Some((file.path.clone(), number.as_str().to_owned()))
            })
            .collect();
        let requests: Vec<PreviewRequest> = paired
            .iter()
            .map(|(path, number)| PreviewRequest {
                source_rel: path,
                number,
                entry: None,
            })
            .collect();

        assert_eq!(requests.len(), 1, "提取不出番号的文件不进入预览");

        // 源文件必须真实存在, 否则计入 stale 而不是预览项.
        std::fs::create_dir_all(dir.path().join("downloads")).expect("应创建目录");
        std::fs::write(dir.path().join("downloads/ABC-123.mp4"), b"v").expect("应写入源");
        let template = Template::parse("{number}.{ext}").expect("应解析成功");
        let preview = super::build_preview(&template, dir.path(), &requests, &BTreeMap::new());
        assert_eq!(preview.items[0].target_path, "ABC-123.mp4");
        assert_eq!(preview.unmatched, 0);
    }
}
