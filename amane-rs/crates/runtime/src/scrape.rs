//! 刮削任务的编排: 番号 → 各来源 → 番号条目.

use std::sync::Arc;

use amane_core::{
    LockableField, MediaNumber, MetadataInput, MetadataRepository, ScrapeWrite, TaskContext,
    TaskError, TaskHandler, TaskKind, TaskLogger, TaskOutcome,
};
use async_trait::async_trait;
use serde::Deserialize;

use amane_crawlers::{Capability, Registry, ScrapedMetadata, SourceError, aggregate};

use crate::resource::ResourceStore;

/// 刮削任务的载荷.
#[derive(Debug, Deserialize)]
struct ScrapePayload {
    number: String,
}

/// 把刮削任务接到来源登记表上.
///
/// 来源并行请求, 结果按登记顺序聚合: 标量取首个非空, 列表整份取首个非空来源.
pub struct ScrapeHandler {
    sources: Arc<Registry>,
    metadata: MetadataRepository,
    resources: Arc<ResourceStore>,
}

impl ScrapeHandler {
    pub fn new(
        sources: Arc<Registry>,
        metadata: MetadataRepository,
        resources: Arc<ResourceStore>,
    ) -> Arc<Self> {
        Arc::new(Self {
            sources,
            metadata,
            resources,
        })
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

        let candidates = self.sources.with_capability(Capability::Metadata);
        if candidates.is_empty() {
            context.logger.error("没有可用的来源").await;
            return Err(TaskError::Failed("没有可用的来源".to_owned()));
        }

        // 并行请求: 单源失败不影响其它来源, 总耗时接近最慢来源.
        // 抓取期间取消要立即生效: 等全部来源返回再检查, 取消会被拖到最后一个超时.
        let fetched = tokio::select! {
            results = futures::future::join_all(candidates.iter().map(|source| async {
                let id = source.descriptor().id;
                (id, source.fetch(&number).await)
            })) => results,
            () = context.cancel.cancelled() => return Err(TaskError::Cancelled),
        };

        let mut hits: Vec<(&str, ScrapedMetadata)> = Vec::new();
        for (id, result) in fetched {
            if context.cancel.is_cancelled() {
                return Err(TaskError::Cancelled);
            }
            match result {
                Ok(scraped) => {
                    context
                        .logger
                        .info(format!(
                            "来源 {id} 命中: {}",
                            scraped.title.as_deref().unwrap_or("无标题")
                        ))
                        .await;
                    hits.push((id, scraped));
                }
                Err(error) => {
                    context
                        .logger
                        .warn(format!("来源 {id} 失败: {error}"))
                        .await;
                    keep_raw(&context.logger, id, &error).await;
                }
            }
        }

        if hits.is_empty() {
            context.logger.error("没有来源返回结果").await;
            return Err(TaskError::Failed("没有来源返回结果".to_owned()));
        }

        let merged = aggregate(&hits);
        let scraped = merged.metadata.clone();
        let input = MetadataInput {
            title: scraped.title.as_deref(),
            plot: scraped.plot.as_deref(),
            release_date: scraped.release_date,
            runtime_minutes: scraped.runtime_minutes,
            maker: scraped.maker.as_deref(),
            label: scraped.label.as_deref(),
            series: scraped.series.as_deref(),
            poster_url: scraped.poster_url.as_deref(),
            actors: Some(&scraped.actors),
        };

        let write = ScrapeWrite {
            number: &number,
            input,
            field_sources: merged.field_sources.clone(),
        };
        let applied = self
            .metadata
            .apply_scraped(&write)
            .await
            .map_err(|error| TaskError::Failed(format!("写入番号条目失败: {error}")))?;
        let row = applied.row;
        context
            .logger
            .info(format!("写入番号条目 {}", row.number))
            .await;

        if !applied.kept.is_empty() {
            let names: Vec<&str> = applied.kept.iter().map(|field| field.label()).collect();
            context
                .logger
                .info(format!("以下字段已锁定, 保留原值: {}", names.join("、")))
                .await;
        }

        // 快照与字段来源用于回溯: 某个字段是从哪个来源来的, 以及各来源当时返回了什么.
        let snapshots: Vec<(String, String)> = hits
            .iter()
            .filter_map(|(id, scraped)| {
                serde_json::to_string(scraped)
                    .ok()
                    .map(|payload| ((*id).to_owned(), payload))
            })
            .collect();
        self.metadata
            .replace_snapshots(row.id, &snapshots)
            .await
            .map_err(|error| TaskError::Failed(format!("写入来源快照失败: {error}")))?;
        // 上游问题 (下载失败) 只告警; 本地写库失败才判任务失败.
        let poster_locked = applied.kept.contains(&LockableField::PosterUrl);
        if poster_locked {
            context.logger.info("海报字段已锁定, 跳过下载").await;
        } else if let Some(url) = row.poster_url.as_deref() {
            context.logger.info("下载海报").await;
            match self.resources.materialize(url).await {
                Ok(resource) => {
                    context
                        .logger
                        .info(format!("海报已存到本地: {}", resource.file_name))
                        .await;
                    self.metadata
                        .set_poster_resource(row.id, resource.id)
                        .await
                        .map_err(|error| TaskError::Failed(format!("关联海报失败: {error}")))?;
                }
                Err(error) => {
                    context.logger.warn(format!("海报下载失败: {error}")).await;
                }
            }
        }

        Ok(TaskOutcome::done())
    }
}

/// 解析失败时把上游原文写入任务目录.
///
/// 站点改版或被拦截时, 一行失败说明不足以定位, 需要检查原文. 留存位置的选择见
/// `docs/dev/tasks.md`.
async fn keep_raw(logger: &TaskLogger, id: &str, error: &SourceError) {
    let Some(raw) = error.raw() else {
        return;
    };

    let name = format!("{id}.raw.{}", raw.extension);
    match logger.write_file(&name, &raw.content).await {
        Ok(()) => logger.info(format!("来源 {id} 的原文已留存: {name}")).await,
        Err(cause) => {
            logger
                .warn(format!("来源 {id} 的原文留存失败: {cause}"))
                .await
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use amane_core::{
        Database, LockableField, MediaNumber, MetadataInput, MetadataRepository, TaskContext,
        TaskError, TaskHandler, TaskLogger, TaskOutcome,
    };
    use async_trait::async_trait;
    use tokio_util::sync::CancellationToken;

    use super::ScrapeHandler;
    use amane_crawlers::{
        Capability, RawBody, Registry, ScrapedMetadata, Source, SourceDescriptor, SourceError,
    };

    struct StubSource {
        descriptor: SourceDescriptor,
        outcome: StubOutcome,
    }

    enum StubOutcome {
        Hit(Option<&'static str>),
        Transport,
        Parse {
            raw: &'static str,
            extension: &'static str,
        },
    }

    #[async_trait]
    impl Source for StubSource {
        fn descriptor(&self) -> &SourceDescriptor {
            &self.descriptor
        }

        async fn fetch(&self, number: &MediaNumber) -> Result<ScrapedMetadata, SourceError> {
            match self.outcome {
                StubOutcome::Hit(title) => Ok(ScrapedMetadata {
                    title: title.map(|title| format!("{title} {number}")),
                    ..ScrapedMetadata::default()
                }),
                StubOutcome::Transport => Err(SourceError::Transport {
                    id: self.descriptor.id,
                    message: "连接超时".to_owned(),
                }),
                StubOutcome::Parse { raw, extension } => Err(SourceError::Parse {
                    id: self.descriptor.id,
                    message: "没有可识别的结构".to_owned(),
                    raw: RawBody {
                        content: raw.to_owned(),
                        extension,
                    },
                }),
            }
        }
    }

    fn stub(id: &'static str, outcome: StubOutcome) -> Box<dyn Source> {
        Box::new(StubSource {
            descriptor: SourceDescriptor {
                id,
                label: id,
                base_url: "https://example.invalid",
                capabilities: &[Capability::Metadata],
            },
            outcome,
        })
    }

    fn source(id: &'static str, title: Option<&'static str>, fails: bool) -> Box<dyn Source> {
        stub(
            id,
            if fails {
                StubOutcome::Transport
            } else {
                StubOutcome::Hit(title)
            },
        )
    }

    fn unparsable(id: &'static str, raw: &'static str, extension: &'static str) -> Box<dyn Source> {
        stub(id, StubOutcome::Parse { raw, extension })
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
        let handler = ScrapeHandler::new(
            Arc::new(Registry::new(sources)),
            metadata.clone(),
            resources,
        );

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
    async fn keeps_locked_field_after_scrape() {
        let fixture = fixture(vec![source("working", Some("刮削"), false)]).await;
        let number = MediaNumber::parse("ABC-123").expect("应解析成功");
        let row = fixture
            .metadata
            .upsert(
                &number,
                &MetadataInput {
                    title: Some("人工标题"),
                    ..MetadataInput::default()
                },
            )
            .await
            .expect("应写入成功");
        fixture
            .metadata
            .set_lock(row.id, LockableField::Title, true)
            .await
            .expect("应加锁成功");

        run(
            &fixture.handler,
            fixture.dir.path(),
            serde_json::json!({ "number": "ABC-123" }),
        )
        .await
        .expect("应完成");

        let stored = fixture
            .metadata
            .find(&number)
            .await
            .expect("应查询成功")
            .expect("应存在");
        assert_eq!(
            stored.title.as_deref(),
            Some("人工标题"),
            "锁定字段不应被刮削覆盖"
        );
    }

    #[tokio::test]
    async fn keeps_raw_of_unparsable_source() {
        let fixture = fixture(vec![
            unparsable("broken", "<html>拦截页</html>", "html"),
            source("working", Some("命中"), false),
        ])
        .await;

        run(
            &fixture.handler,
            fixture.dir.path(),
            serde_json::json!({ "number": "ABC-123" }),
        )
        .await
        .expect("单个来源解析失败不应让任务失败");

        let path = fixture.dir.path().join("task-1/broken.raw.html");
        let content = tokio::fs::read_to_string(&path).await.expect("应留存原文");
        assert_eq!(content, "<html>拦截页</html>");

        let lines = TaskLogger::read(fixture.dir.path(), 1)
            .await
            .expect("应读到日志");
        let texts: Vec<&str> = lines.iter().map(|line| line.message.as_str()).collect();
        assert!(
            texts.iter().any(|text| text.contains("broken.raw.html")),
            "日志应指出原文位置, 实际 {texts:?}"
        );
    }

    #[tokio::test]
    async fn keeps_raw_when_every_source_is_unparsable() {
        let fixture = fixture(vec![unparsable("broken", "{\"errors\":[]}", "json")]).await;

        let error = run(
            &fixture.handler,
            fixture.dir.path(),
            serde_json::json!({ "number": "ABC-123" }),
        )
        .await
        .expect_err("全部解析失败时任务应失败");

        assert!(error.to_string().contains("没有来源返回结果"));
        assert!(
            !error.to_string().contains("errors"),
            "失败说明不应携带原文, 实际 {error}"
        );
        assert!(
            fixture.dir.path().join("task-1/broken.raw.json").exists(),
            "没有写入番号条目时也要留存原文"
        );
    }

    #[tokio::test]
    async fn fails_when_no_source_registered() {
        let fixture = fixture(Vec::new()).await;

        let result = run(
            &fixture.handler,
            fixture.dir.path(),
            serde_json::json!({ "number": "ABC-123" }),
        )
        .await;

        match result {
            Err(TaskError::Failed(message)) => assert!(message.contains("没有可用的来源")),
            _ => panic!("没有来源时必须失败并说明原因"),
        }
    }

    #[tokio::test]
    async fn fails_when_every_source_errors() {
        let fixture = fixture(vec![
            source("a", Some("忽略"), true),
            source("b", Some("忽略"), true),
        ])
        .await;

        let result = run(
            &fixture.handler,
            fixture.dir.path(),
            serde_json::json!({ "number": "ABC-123" }),
        )
        .await;

        match result {
            Err(TaskError::Failed(message)) => assert!(message.contains("没有来源返回结果")),
            _ => panic!("所有来源失败时必须失败并说明原因"),
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
