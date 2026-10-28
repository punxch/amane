//! 整理任务: 按预览落点移动文件; 冲突按预览时的选择处置, 结果逐项写入日志.

use std::collections::BTreeMap;
use std::sync::Arc;

use amane_core::organize::{ConflictAction, ExecuteRequest, ItemOutcome};
use amane_core::{
    OrganizeError, OrganizeRepository, TaskContext, TaskError, TaskHandler, TaskKind, TaskOutcome,
};
use arc_swap::ArcSwap;
use async_trait::async_trait;
use serde::Deserialize;

use amane_core::config::Settings;

/// 整理任务的载荷. choices 以源相对路径为键, 值是 `overwrite` / `rename`; 缺省跳过冲突项.
#[derive(Debug, Deserialize)]
struct OrganizePayload {
    library_id: i64,
    #[serde(default)]
    choices: BTreeMap<String, String>,
}

/// 把整理任务接到库登记、整理登记与配置快照上.
pub struct OrganizeHandler {
    libraries: amane_core::LibraryRepository,
    metadata: amane_core::MetadataRepository,
    plans: OrganizeRepository,
    settings: Arc<ArcSwap<Settings>>,
}

impl OrganizeHandler {
    pub fn new(
        libraries: amane_core::LibraryRepository,
        metadata: amane_core::MetadataRepository,
        plans: OrganizeRepository,
        settings: Arc<ArcSwap<Settings>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            libraries,
            metadata,
            plans,
            settings,
        })
    }
}

#[async_trait]
impl TaskHandler for OrganizeHandler {
    fn kind(&self) -> TaskKind {
        TaskKind::Organize
    }

    async fn run(&self, context: TaskContext) -> Result<TaskOutcome, TaskError> {
        let payload: OrganizePayload = serde_json::from_value(context.payload.clone())
            .map_err(|error| TaskError::Failed(format!("整理载荷不合法: {error}")))?;
        let root = self
            .libraries
            .find(payload.library_id)
            .await
            .map_err(|error| TaskError::Failed(format!("查询媒体库失败: {error}")))?
            .ok_or_else(|| TaskError::Failed(format!("媒体库 {} 不存在", payload.library_id)))?;

        let settings = self.settings.load_full();
        let template = amane_core::Template::parse(&settings.hot.organize.video_template)
            .map_err(|error| TaskError::Failed(format!("路径模板无法解析: {error}")))?;

        let (_, media) = self
            .libraries
            .files(
                payload.library_id,
                Some(amane_core::ScanKind::Media),
                200,
                0,
            )
            .await
            .map_err(|error| TaskError::Failed(format!("读取扫描索引失败: {error}")))?;

        // 与预览同一套配对与渲染: 落点一致由「同一份输入 + 同一函数」保证.
        let mut numbers: Vec<(usize, amane_core::MediaNumber)> = Vec::new();
        let mut unmatched = 0i64;
        for (index, file) in media.iter().enumerate() {
            match amane_core::number_from_path(&file.path) {
                Some(number) => numbers.push((index, number)),
                None => unmatched += 1,
            }
        }
        let metadata = &self.metadata;
        let entries: Vec<Option<amane_core::Metadata>> = futures::future::join_all(
            numbers
                .iter()
                .map(|(_, number)| async move { metadata.find(number).await.ok().flatten() }),
        )
        .await;

        let mut preview_requests: Vec<amane_core::PreviewRequest> =
            Vec::with_capacity(numbers.len());
        for ((index, number), entry) in numbers.iter().zip(&entries) {
            preview_requests.push(amane_core::PreviewRequest {
                source_rel: &media[*index].path,
                number: number.as_str(),
                entry: entry.as_ref(),
            });
        }

        let plans: BTreeMap<String, String> = self
            .plans
            .list_for_root(root.id)
            .await
            .map_err(|error| TaskError::Failed(format!("读取整理登记失败: {error}")))?
            .into_iter()
            .map(|plan| (plan.path, plan.number))
            .collect();

        let preview =
            amane_core::build_preview(&template, &root.path_buf(), &preview_requests, &plans);

        // 处置选择: 预览标出的冲突项没有明确选择时按跳过处理, 不得默默覆盖.
        let requests: Vec<ExecuteRequest> = preview
            .items
            .iter()
            .map(|item| {
                let action = if item.conflict.is_some() {
                    payload
                        .choices
                        .get(&item.source_path)
                        .and_then(|value| ConflictAction::parse(value))
                        .unwrap_or(ConflictAction::Skip)
                } else {
                    ConflictAction::Skip
                };
                ExecuteRequest {
                    source_rel: item.source_path.clone(),
                    target_rel: item.target_path.clone(),
                    number: item.number.clone(),
                    action,
                }
            })
            .collect();

        context
            .logger
            .info(format!(
                "开始整理 {}: {} 个文件, {} 个提取不出番号",
                root.path,
                requests.len(),
                unmatched
            ))
            .await;

        // 移动是纯阻塞 IO, 交给阻塞线程池.
        let root_path = root.path_buf();
        let subtitle_extensions: Vec<String> = settings.hot.organize.subtitle_extensions.clone();
        let create_strm = settings.hot.organize.create_strm;
        let (report, failures) = tokio::task::spawn_blocking(move || {
            amane_core::execute_batch(&root_path, &requests, &subtitle_extensions, create_strm)
        })
        .await
        .map_err(|error| TaskError::Failed(format!("整理线程异常退出: {error}")))?;

        // 成功与跳过分开记录, 每个文件的处理结果可见.
        for outcome in &report.moved {
            let ItemOutcome::Moved {
                target,
                subtitles,
                strm,
            } = outcome
            else {
                continue;
            };
            context.logger.info(format!("已移动 → {target}")).await;
            for subtitle in subtitles {
                context.logger.info(format!("字幕随配 → {subtitle}")).await;
            }
            if let Some(strm) = strm {
                context.logger.info(format!("STRM 已生成 → {strm}")).await;
            }
        }
        for (source, reason) in &failures {
            context
                .logger
                .warn(format!("跳过 {source}: {reason}"))
                .await;
        }

        // 登记成功项: 预览判「同名不同番号」与后续清理都依赖这张表.
        let mut registered = 0i64;
        for stub in &report.plans {
            match self
                .plans
                .register(root.id, &stub.path, &stub.number, &stub.source_path)
                .await
            {
                Ok(_) => registered += 1,
                Err(OrganizeError::PathTaken(path)) => {
                    context
                        .logger
                        .warn(format!("登记 {path} 失败: 路径已被占用"))
                        .await;
                }
                Err(OrganizeError::Database(error)) => {
                    return Err(TaskError::Failed(format!("写入整理登记失败: {error}")));
                }
            }
        }
        context
            .logger
            .info(format!(
                "整理完成: 移动 {}、跳过 {}、登记 {}",
                report.moved.len(),
                failures.len(),
                registered
            ))
            .await;

        Ok(TaskOutcome::done())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::Arc;

    use amane_core::config::{OrganizeSettings, Settings};
    use amane_core::library::LibraryRepository;
    use amane_core::organize::{OrganizeRepository, TRASH_DIRNAME};
    use amane_core::{
        Database, MetadataRepository, TaskContext, TaskError, TaskHandler, TaskKind, TaskLogger,
        TaskOutcome,
    };
    use arc_swap::ArcSwap;
    use tokio_util::sync::CancellationToken;

    use super::OrganizeHandler;

    async fn fixture() -> (tempfile::TempDir, Arc<OrganizeHandler>) {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let database = Database::open(&dir.path().join("amane.db"))
            .await
            .expect("应打开数据库");
        let libraries = LibraryRepository::new(database.clone());
        let metadata = MetadataRepository::new(database.clone());
        let plans = OrganizeRepository::new(database);
        let settings = Arc::new(ArcSwap::from_pointee(Settings::default()));
        let handler = OrganizeHandler::new(libraries, metadata, plans, settings);
        (dir, handler)
    }

    async fn run(
        handler: &OrganizeHandler,
        dir: &std::path::Path,
        payload: serde_json::Value,
    ) -> Result<TaskOutcome, TaskError> {
        let logger = TaskLogger::open(dir, 1).await.expect("应创建日志");
        handler
            .run(TaskContext {
                task_id: 1,
                payload,
                logger,
                cancel: CancellationToken::new(),
            })
            .await
    }

    #[tokio::test]
    async fn organizes_files_and_registers_plans() {
        let (dir, handler) = fixture().await;
        let library = dir.path().join("library");
        fs::create_dir_all(&library).expect("应创建库目录");
        let database = Database::open(&dir.path().join("amane.db"))
            .await
            .expect("应打开数据库");
        let libraries = LibraryRepository::new(database.clone());
        let root = libraries.create(&library).await.expect("应登记成功");
        fs::write(library.join("ABC-123.mp4"), b"v").expect("应写入视频");
        fs::write(library.join("ABC-123.srt"), b"sub").expect("应写入字幕");
        libraries
            .replace_files(
                root.id,
                &[amane_core::LibraryFile::new(
                    "ABC-123.mp4",
                    amane_core::ScanKind::Media,
                )],
                &[],
            )
            .await
            .expect("应写入索引");

        run(
            &handler,
            dir.path(),
            serde_json::json!({"library_id": root.id}),
        )
        .await
        .expect("应完成");

        // 默认模板 {maker}/{number}/{number}.{ext}: maker 空折叠为 ABC-123/ABC-123.mp4.
        assert!(
            library.join("ABC-123/ABC-123.mp4").exists(),
            "落点应与预览一致"
        );
        assert!(library.join("ABC-123/ABC-123.srt").exists(), "字幕应随配");
        assert!(!library.join("ABC-123.mp4").exists(), "源应消失");

        // 登记写入: 预览判「同名不同番号」依赖它.
        let plan = OrganizeRepository::new(
            Database::open(&dir.path().join("amane.db"))
                .await
                .expect("应打开数据库"),
        )
        .find_by_path(root.id, "ABC-123/ABC-123.mp4")
        .await
        .expect("应查询成功");
        assert!(plan.is_some(), "成功项应登记");
    }

    #[tokio::test]
    async fn conflicts_without_choice_are_skipped() {
        let (dir, handler) = fixture().await;
        let library = dir.path().join("library");
        fs::create_dir_all(library.join("ABC-123")).expect("应创建目录");
        let database = Database::open(&dir.path().join("amane.db"))
            .await
            .expect("应打开数据库");
        let libraries = LibraryRepository::new(database.clone());
        let root = libraries.create(&library).await.expect("应登记成功");
        fs::write(library.join("ABC-123.mp4"), b"new").expect("应写入源");
        fs::write(library.join("ABC-123/ABC-123.mp4"), b"old").expect("应写入占用者");
        libraries
            .replace_files(
                root.id,
                &[amane_core::LibraryFile::new(
                    "ABC-123.mp4",
                    amane_core::ScanKind::Media,
                )],
                &[],
            )
            .await
            .expect("应写入索引");

        // 无 choices: 冲突项跳过, 占用者完好.
        run(
            &handler,
            dir.path(),
            serde_json::json!({"library_id": root.id}),
        )
        .await
        .expect("应完成");
        assert_eq!(
            fs::read(library.join("ABC-123/ABC-123.mp4")).expect("占用者应完好"),
            b"old"
        );

        // overwrite: 旧文件进回收目录, 可恢复.
        run(
            &handler,
            dir.path(),
            serde_json::json!({
                "library_id": root.id,
                "choices": {"ABC-123.mp4": "overwrite"}
            }),
        )
        .await
        .expect("应完成");
        assert_eq!(
            fs::read(library.join("ABC-123/ABC-123.mp4")).expect("应读到新文件"),
            b"new"
        );
        assert!(
            library
                .join(TRASH_DIRNAME)
                .join("ABC-123/ABC-123.mp4")
                .exists(),
            "覆盖前旧文件应进入回收目录"
        );
    }

    #[tokio::test]
    async fn strm_generation_follows_config() {
        let (dir, _handler) = fixture().await;
        let library = dir.path().join("library");
        fs::create_dir_all(&library).expect("应创建库目录");
        let database = Database::open(&dir.path().join("amane.db"))
            .await
            .expect("应打开数据库");
        let libraries = LibraryRepository::new(database.clone());
        let plans = OrganizeRepository::new(database.clone());
        let metadata = MetadataRepository::new(database);
        let root = libraries.create(&library).await.expect("应登记成功");
        fs::write(library.join("ABC-123.mp4"), b"v").expect("应写入视频");
        libraries
            .replace_files(
                root.id,
                &[amane_core::LibraryFile::new(
                    "ABC-123.mp4",
                    amane_core::ScanKind::Media,
                )],
                &[],
            )
            .await
            .expect("应写入索引");

        // 开启 STRM 后执行.
        let settings = Arc::new(ArcSwap::from_pointee(Settings {
            hot: amane_core::config::HotSettings {
                organize: OrganizeSettings {
                    create_strm: true,
                    ..OrganizeSettings::default()
                },
                ..amane_core::config::HotSettings::default()
            },
            ..Settings::default()
        }));
        let handler = OrganizeHandler::new(libraries, metadata, plans, settings);

        run(
            &handler,
            dir.path(),
            serde_json::json!({"library_id": root.id}),
        )
        .await
        .expect("应完成");

        let body = fs::read_to_string(library.join("ABC-123/ABC-123.strm")).expect("STRM 应存在");
        assert!(
            body.trim_end().ends_with("ABC-123/ABC-123.mp4"),
            "正文应是完整路径, 实际 {body}"
        );
    }

    #[tokio::test]
    async fn malformed_payload_fails_with_reason() {
        let (dir, handler) = fixture().await;

        let result = run(&handler, dir.path(), serde_json::json!({"id": 1})).await;

        match result {
            Err(TaskError::Failed(message)) => {
                assert!(message.contains("载荷不合法"), "实际 {message}")
            }
            other => panic!("载荷缺字段必须失败, 实际 {other:?}"),
        }
    }

    #[tokio::test]
    async fn kind_is_organize() {
        let (_dir, handler) = fixture().await;
        assert_eq!(handler.kind(), TaskKind::Organize);
    }
}
