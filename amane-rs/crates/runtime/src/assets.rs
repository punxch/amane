//! 资源下载任务: 刮削完成后单独拉取海报与封面.

use std::sync::Arc;

use amane_core::{
    LockableField, MediaNumber, MetadataRepository, TaskContext, TaskError, TaskHandler, TaskKind,
    TaskOutcome, TaskPayload,
};
use async_trait::async_trait;

use crate::resource::ResourceStore;

/// 两类图: 竖版海报与横版封面. 各自的登记位置不同, 其余流程一致.
#[derive(Debug, Clone, Copy)]
enum ImageKind {
    Poster,
    Cover,
}

impl ImageKind {
    fn label(self) -> &'static str {
        match self {
            Self::Poster => "海报",
            Self::Cover => "封面",
        }
    }
}

/// 把条目的海报与封面取到本地.
///
/// 作为刮削的后继任务: 下载失败不该让刮削判失败, 而拆开之后失败有独立的任务记录与重试入口.
pub struct AssetsHandler {
    metadata: MetadataRepository,
    resources: Arc<ResourceStore>,
}

impl AssetsHandler {
    pub fn new(metadata: MetadataRepository, resources: Arc<ResourceStore>) -> Arc<Self> {
        Arc::new(Self {
            metadata,
            resources,
        })
    }

    async fn store_image(
        &self,
        context: &TaskContext,
        metadata_id: i64,
        url: &str,
        kind: ImageKind,
    ) -> Result<(), TaskError> {
        let label = kind.label();
        context.logger.info(format!("下载{label}")).await;

        let resource = match self.resources.materialize(url).await {
            Ok(resource) => resource,
            Err(error) => {
                context
                    .logger
                    .warn(format!("{label}下载失败: {error}"))
                    .await;
                return Ok(());
            }
        };
        context
            .logger
            .info(format!("{label}已存到本地: {}", resource.file_name))
            .await;

        let linked = match kind {
            ImageKind::Poster => {
                self.metadata
                    .set_poster_resource(metadata_id, resource.id)
                    .await
            }
            ImageKind::Cover => {
                self.metadata
                    .set_cover_resource(metadata_id, resource.id)
                    .await
            }
        };
        linked.map_err(|error| TaskError::Failed(format!("关联{label}失败: {error}")))
    }
}

#[async_trait]
impl TaskHandler for AssetsHandler {
    fn kind(&self) -> TaskKind {
        TaskKind::Assets
    }

    async fn run(&self, context: TaskContext) -> Result<TaskOutcome, TaskError> {
        let TaskPayload::Assets { number } = &context.payload else {
            return Err(TaskError::Failed("载荷类型与任务类型不匹配".to_owned()));
        };
        let number = MediaNumber::parse(number)
            .map_err(|error| TaskError::Failed(format!("番号无法识别: {error}")))?;

        let row = self
            .metadata
            .find(&number)
            .await
            .map_err(|error| TaskError::Failed(format!("查询番号条目失败: {error}")))?
            .ok_or_else(|| TaskError::Failed(format!("影片 {number} 不存在")))?;

        let locks = self
            .metadata
            .locks(row.id)
            .await
            .map_err(|error| TaskError::Failed(format!("读取字段锁失败: {error}")))?;

        if locks.contains(&LockableField::PosterUrl) {
            context.logger.info("海报字段已锁定, 跳过下载").await;
        } else if let Some(url) = row.poster_url.as_deref() {
            self.store_image(&context, row.id, url, ImageKind::Poster)
                .await?;
        }

        if let Some(url) = row.cover_url.as_deref() {
            self.store_image(&context, row.id, url, ImageKind::Cover)
                .await?;
        }

        Ok(TaskOutcome::done())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use amane_core::{
        Database, LockableField, MediaNumber, MetadataInput, MetadataRepository, TaskContext,
        TaskError, TaskHandler, TaskLogger, TaskOutcome, TaskPayload,
    };
    use tokio_util::sync::CancellationToken;

    use super::AssetsHandler;

    struct Fixture {
        dir: tempfile::TempDir,
        handler: Arc<AssetsHandler>,
        metadata: MetadataRepository,
    }

    async fn fixture() -> Fixture {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let database = Database::open(&dir.path().join("amane.db"))
            .await
            .expect("应打开数据库");
        let metadata = MetadataRepository::new(database.clone());
        let client = Arc::new(
            amane_net::WebClient::new(
                std::time::Duration::from_secs(5),
                amane_net::RateLimiters::new(amane_net::Rate::per_second(1, 1.0)),
            )
            .expect("客户端应构造成功"),
        );
        let resources = Arc::new(crate::resource::ResourceStore::new(
            dir.path().join("resources"),
            amane_core::ResourceRepository::new(database),
            client,
        ));
        let handler = AssetsHandler::new(metadata.clone(), resources);

        Fixture {
            dir,
            handler,
            metadata,
        }
    }

    async fn run(
        handler: &AssetsHandler,
        dir: &std::path::Path,
        payload: TaskPayload,
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
    async fn download_failure_keeps_entry_and_reports_warning() {
        let fixture = fixture().await;
        let number = MediaNumber::parse("ABC-123").expect("应解析成功");
        fixture
            .metadata
            .upsert(
                &number,
                &MetadataInput {
                    poster_url: Some("https://example.invalid/poster.jpg"),
                    cover_url: Some("https://example.invalid/cover.jpg"),
                    ..MetadataInput::default()
                },
            )
            .await
            .expect("应写入成功");

        let outcome = run(
            &fixture.handler,
            fixture.dir.path(),
            TaskPayload::Assets {
                number: "ABC-123".to_owned(),
            },
        )
        .await
        .expect("下载失败不应让任务失败");

        assert!(outcome.followups.is_empty());
        let row = fixture
            .metadata
            .find(&number)
            .await
            .expect("应查询成功")
            .expect("条目应仍在");
        assert!(row.poster_resource_id.is_none(), "下载失败不应有本地资源");
        assert!(row.cover_resource_id.is_none());

        let lines = TaskLogger::read(fixture.dir.path(), 1)
            .await
            .expect("应读到日志");
        assert!(
            lines.iter().any(|line| line.message.contains("下载失败")),
            "失败应写进任务日志"
        );
    }

    #[tokio::test]
    async fn locked_poster_is_skipped() {
        let fixture = fixture().await;
        let number = MediaNumber::parse("ABC-123").expect("应解析成功");
        let row = fixture
            .metadata
            .upsert(
                &number,
                &MetadataInput {
                    poster_url: Some("https://example.invalid/poster.jpg"),
                    ..MetadataInput::default()
                },
            )
            .await
            .expect("应写入成功");
        fixture
            .metadata
            .set_lock(row.id, LockableField::PosterUrl, true)
            .await
            .expect("应加锁成功");

        run(
            &fixture.handler,
            fixture.dir.path(),
            TaskPayload::Assets {
                number: "ABC-123".to_owned(),
            },
        )
        .await
        .expect("应完成");

        let lines = TaskLogger::read(fixture.dir.path(), 1)
            .await
            .expect("应读到日志");
        assert!(
            lines.iter().any(|line| line.message.contains("已锁定")),
            "锁定字段应跳过下载"
        );
    }

    #[tokio::test]
    async fn missing_entry_fails_with_reason() {
        let fixture = fixture().await;

        let result = run(
            &fixture.handler,
            fixture.dir.path(),
            TaskPayload::Assets {
                number: "ZZZ-999".to_owned(),
            },
        )
        .await;

        match result {
            Err(TaskError::Failed(message)) => assert!(message.contains("不存在")),
            other => panic!("条目不存在时应失败, 实际 {other:?}"),
        }
    }
}
