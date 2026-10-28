//! 整理预览的装配: 取媒体文件、提番号、查条目、读登记、渲染目标与冲突.
//!
//! 预览有两条入口 (HTTP 端点与整理任务), 装配只此一份: 两侧交出的都是同一组仓储,
//! 拿回的都是同一个 `OrganizePreview`.

use std::collections::BTreeMap;

use thiserror::Error;

use super::aggregate::{OrganizePreview, PreviewRequest, build_preview, number_from_path};
use super::repository::{OrganizeError, OrganizeRepository};
use super::template::{Template, TemplateError};
use crate::library::{LibraryError, LibraryRepository, LibraryRoot, ScanKind};
use crate::metadata::{Metadata, MetadataRepository, RepositoryError};

#[derive(Debug, Error)]
pub enum PreviewError {
    #[error("路径模板无法解析: {0}")]
    Template(#[from] TemplateError),
    #[error("读取扫描索引失败: {0}")]
    Library(#[from] LibraryError),
    #[error("查询番号条目失败: {0}")]
    Metadata(#[from] RepositoryError),
    #[error("读取整理登记失败: {0}")]
    Plans(#[from] OrganizeError),
}

/// 把扫描索引与条目装配成一次整理预览.
#[derive(Clone)]
pub struct PreviewAssembler {
    libraries: LibraryRepository,
    metadata: MetadataRepository,
    plans: OrganizeRepository,
}

impl PreviewAssembler {
    pub fn new(
        libraries: LibraryRepository,
        metadata: MetadataRepository,
        plans: OrganizeRepository,
    ) -> Self {
        Self {
            libraries,
            metadata,
            plans,
        }
    }

    /// 装配一个媒体库的预览.
    ///
    /// 只读: 不创建目录也不移动文件, 重复调用结果一致. 条目查询失败必须上抛 ——
    /// 当成「条目缺席」会把预览渲染成另一副样子, 界面上看不出区别.
    pub async fn assemble(
        &self,
        root: &LibraryRoot,
        video_template: &str,
    ) -> Result<OrganizePreview, PreviewError> {
        let template = Template::parse(video_template)?;

        // 扫描索引里的媒体文件是预览的全部对象: 按页取会让超出页大小的文件既不预览也不提示.
        let media = self
            .libraries
            .all_files(root.id, Some(ScanKind::Media))
            .await?;

        // 逐文件提取番号; 提取不出的计入 unmatched, 不进入预览.
        let mut unmatched = 0i64;
        let mut numbers: Vec<(usize, crate::number::MediaNumber)> = Vec::new();
        for (index, file) in media.iter().enumerate() {
            match number_from_path(&file.path) {
                Some(number) => numbers.push((index, number)),
                None => unmatched += 1,
            }
        }

        // 条目查询并行发出: 读池多连接, 一次配齐全部条目.
        let metadata = &self.metadata;
        let entries: Vec<Option<Metadata>> = futures::future::join_all(
            numbers
                .iter()
                .map(|(_, number)| async move { metadata.find(number).await }),
        )
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;

        let mut requests: Vec<PreviewRequest> = Vec::with_capacity(numbers.len());
        for ((index, number), entry) in numbers.iter().zip(&entries) {
            requests.push(PreviewRequest {
                source_rel: &media[*index].path,
                number: number.as_str(),
                entry: entry.as_ref(),
            });
        }

        let plans: BTreeMap<String, String> = self
            .plans
            .list_for_root(root.id)
            .await?
            .into_iter()
            .map(|plan| (plan.path, plan.number))
            .collect();

        let mut preview = build_preview(&template, &root.path_buf(), &requests, &plans);
        // `build_preview` 只看到已经带番号的请求, 提不出番号的那部分由这里补上.
        preview.unmatched += unmatched;

        Ok(preview)
    }
}

#[cfg(test)]
mod tests {
    use super::{PreviewAssembler, PreviewError};
    use crate::db::Database;
    use crate::library::{LibraryFile, LibraryRepository, ScanKind};
    use crate::metadata::{MetadataInput, MetadataRepository};
    use crate::number::MediaNumber;
    use crate::organize::OrganizeRepository;

    async fn fixture() -> (
        tempfile::TempDir,
        PreviewAssembler,
        LibraryRepository,
        MetadataRepository,
    ) {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let database = Database::open(&dir.path().join("amane.db"))
            .await
            .expect("应打开数据库");
        let libraries = LibraryRepository::new(database.clone());
        let metadata = MetadataRepository::new(database.clone());
        let plans = OrganizeRepository::new(database);
        let assembler = PreviewAssembler::new(libraries.clone(), metadata.clone(), plans);

        (dir, assembler, libraries, metadata)
    }

    #[tokio::test]
    async fn counts_files_without_a_number() {
        let (dir, assembler, libraries, _metadata) = fixture().await;
        let root = libraries.create(dir.path()).await.expect("应登记成功");
        libraries
            .replace_files(
                root.id,
                &[
                    LibraryFile::new("ABC-123.mp4", ScanKind::Media),
                    LibraryFile::new("not-a-number.mp4", ScanKind::Media),
                ],
                &[],
            )
            .await
            .expect("应写入成功");
        std::fs::write(dir.path().join("ABC-123.mp4"), b"v").expect("应写入视频");

        let preview = assembler
            .assemble(&root, "{number}/{number}.{ext}")
            .await
            .expect("应装配成功");

        assert_eq!(preview.unmatched, 1, "提不出番号的文件单独计数");
        assert_eq!(preview.items.len(), 1);
    }

    #[tokio::test]
    async fn renders_target_from_entry() {
        let (dir, assembler, libraries, metadata) = fixture().await;
        let root = libraries.create(dir.path()).await.expect("应登记成功");
        libraries
            .replace_files(
                root.id,
                &[LibraryFile::new("ABC-123.mp4", ScanKind::Media)],
                &[],
            )
            .await
            .expect("应写入成功");
        std::fs::write(dir.path().join("ABC-123.mp4"), b"v").expect("应写入视频");
        let number = MediaNumber::parse("ABC-123").expect("应解析成功");
        metadata
            .upsert(
                &number,
                &MetadataInput {
                    maker: Some("SOD"),
                    ..MetadataInput::default()
                },
            )
            .await
            .expect("应写入成功");

        let preview = assembler
            .assemble(&root, "{maker}/{number}.{ext}")
            .await
            .expect("应装配成功");

        assert_eq!(preview.items.len(), 1);
        assert_eq!(preview.items[0].target_path, "SOD/ABC-123.mp4");
    }

    #[tokio::test]
    async fn rejects_unparsable_template() {
        let (dir, assembler, libraries, _metadata) = fixture().await;
        let root = libraries.create(dir.path()).await.expect("应登记成功");

        let result = assembler.assemble(&root, "{unknown}/{number}").await;

        assert!(
            matches!(result, Err(PreviewError::Template(_))),
            "模板不合法应在装配前被拒绝"
        );
    }
}
