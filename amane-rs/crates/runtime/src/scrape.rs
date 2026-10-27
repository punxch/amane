//! 刮削任务的编排: 番号 → 各来源 → 番号条目.

use std::sync::Arc;

use amane_core::{
    MediaNumber, MetadataRepository, TaskContext, TaskError, TaskHandler, TaskKind, TaskOutcome,
};
use async_trait::async_trait;
use serde::Deserialize;

use amane_crawlers::{Capability, Registry};

/// 刮削任务的载荷.
#[derive(Debug, Deserialize)]
struct ScrapePayload {
    number: String,
}

/// 把刮削任务接到来源登记表上.
///
/// 首个切片只做「首个成功来源获胜」: 多源按字段合并属于聚合切片, 这里不提前实现.
pub struct ScrapeHandler {
    sources: Arc<Registry>,
    metadata: MetadataRepository,
}

impl ScrapeHandler {
    pub fn new(sources: Arc<Registry>, metadata: MetadataRepository) -> Arc<Self> {
        Arc::new(Self { sources, metadata })
    }
}

#[async_trait]
impl TaskHandler for ScrapeHandler {
    fn kind(&self) -> TaskKind {
        TaskKind::Scrape
    }

    async fn run(&self, context: TaskContext) -> Result<TaskOutcome, TaskError> {
        let payload: ScrapePayload = serde_json::from_value(context.payload.clone())
            .map_err(|error| TaskError::Failed(format!("刮削载荷不合法: {error}")))?;
        let number = MediaNumber::parse(&payload.number)
            .map_err(|error| TaskError::Failed(format!("番号无法识别: {error}")))?;

        context.logger.info(format!("开始刮削 {number}")).await;

        let mut hits = 0;
        let mut title = None;
        for source in self.sources.with_capability(Capability::Metadata) {
            if context.cancel.is_cancelled() {
                return Err(TaskError::Cancelled);
            }

            let id = source.descriptor().id;
            context.logger.info(format!("请求来源 {id}")).await;
            match source.fetch(&number).await {
                Ok(scraped) => {
                    hits += 1;
                    context
                        .logger
                        .info(format!(
                            "来源 {id} 命中: {}",
                            scraped.title.as_deref().unwrap_or("无标题")
                        ))
                        .await;
                    if title.is_none() {
                        title = scraped.title;
                    }
                }
                Err(error) => {
                    context
                        .logger
                        .warn(format!("来源 {id} 失败: {error}"))
                        .await;
                }
            }
        }

        if hits == 0 {
            context.logger.error("没有来源返回结果").await;
            return Err(TaskError::Failed("没有来源返回结果".to_owned()));
        }

        let row = self
            .metadata
            .upsert(&number, title.as_deref())
            .await
            .map_err(|error| TaskError::Failed(format!("写入番号条目失败: {error}")))?;
        context
            .logger
            .info(format!("写入番号条目 {}", row.number))
            .await;

        Ok(TaskOutcome::done())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use amane_core::{
        Database, MediaNumber, MetadataRepository, TaskContext, TaskError, TaskHandler, TaskLogger,
        TaskOutcome,
    };
    use async_trait::async_trait;
    use tokio_util::sync::CancellationToken;

    use super::ScrapeHandler;
    use amane_crawlers::{
        Capability, Registry, ScrapedMetadata, Source, SourceDescriptor, SourceError,
    };

    struct StubSource {
        descriptor: SourceDescriptor,
        title: Option<&'static str>,
        fails: bool,
    }

    #[async_trait]
    impl Source for StubSource {
        fn descriptor(&self) -> &SourceDescriptor {
            &self.descriptor
        }

        async fn fetch(&self, number: &MediaNumber) -> Result<ScrapedMetadata, SourceError> {
            if self.fails {
                return Err(SourceError::Transport {
                    id: self.descriptor.id,
                    message: "连接超时".to_owned(),
                });
            }
            Ok(ScrapedMetadata {
                title: self.title.map(|title| format!("{title} {number}")),
                ..ScrapedMetadata::default()
            })
        }
    }

    fn source(id: &'static str, title: Option<&'static str>, fails: bool) -> Box<dyn Source> {
        Box::new(StubSource {
            descriptor: SourceDescriptor {
                id,
                label: id,
                base_url: "https://example.invalid",
                capabilities: &[Capability::Metadata],
            },
            title,
            fails,
        })
    }

    struct Fixture {
        dir: tempfile::TempDir,
        handler: Arc<ScrapeHandler>,
        metadata: MetadataRepository,
    }

    async fn fixture(sources: Vec<Box<dyn Source>>) -> Fixture {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let database = Database::open(&dir.path().join("amane.db"))
            .await
            .expect("应打开数据库");
        let metadata = MetadataRepository::new(database);
        let handler = ScrapeHandler::new(Arc::new(Registry::new(sources)), metadata.clone());

        Fixture {
            dir,
            handler,
            metadata,
        }
    }

    async fn run(
        handler: &ScrapeHandler,
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
    async fn writes_entry_from_first_hit() {
        let fixture = fixture(vec![
            source("failing", Some("忽略"), true),
            source("working", Some("命中"), false),
        ])
        .await;

        let outcome = run(
            &fixture.handler,
            fixture.dir.path(),
            serde_json::json!({ "number": "ABC-123" }),
        )
        .await
        .expect("应完成");

        assert!(outcome.followups.is_empty());
        let number = MediaNumber::parse("ABC-123").expect("应解析成功");
        let row = fixture
            .metadata
            .find(&number)
            .await
            .expect("应查询成功")
            .expect("应写入番号条目");
        assert_eq!(row.title.as_deref(), Some("命中 ABC-123"));
    }

    #[tokio::test]
    async fn fails_when_no_source_answers() {
        let fixture = fixture(Vec::new()).await;

        let result = run(
            &fixture.handler,
            fixture.dir.path(),
            serde_json::json!({ "number": "ABC-123" }),
        )
        .await;

        match result {
            Err(TaskError::Failed(message)) => assert!(message.contains("没有来源返回结果")),
            _ => panic!("无来源时必须失败并说明原因"),
        }
    }

    #[tokio::test]
    async fn rejects_malformed_payload() {
        let fixture = fixture(vec![source("working", Some("命中"), false)]).await;

        let result = run(
            &fixture.handler,
            fixture.dir.path(),
            serde_json::json!({ "code": 1 }),
        )
        .await;

        match result {
            Err(TaskError::Failed(message)) => assert!(message.contains("载荷不合法")),
            _ => panic!("载荷缺字段时必须失败并说明原因"),
        }
    }

    #[tokio::test]
    async fn rejects_unrecognized_number() {
        let fixture = fixture(vec![source("working", Some("命中"), false)]).await;

        let result = run(
            &fixture.handler,
            fixture.dir.path(),
            serde_json::json!({ "number": "???" }),
        )
        .await;

        match result {
            Err(TaskError::Failed(message)) => assert!(message.contains("番号无法识别")),
            _ => panic!("非法番号必须失败并说明原因"),
        }
    }
}
