//! 刮削任务的编排: 番号 → 各来源 → 番号条目.

use std::sync::Arc;

use amane_core::{
    MediaNumber, MetadataInput, MetadataRepository, ScrapeWrite, SourceSnapshotInput, TaskContext,
    TaskError, TaskHandler, TaskKind, TaskLogger, TaskOutcome, TaskPayload, TaskSpec,
};
use async_trait::async_trait;
use tracing::Instrument;

use amane_crawlers::{Registry, ScrapedMetadata, SourceError, aggregate};

/// 把刮削任务接到来源登记表上.
///
/// 来源并行请求, 结果按登记顺序聚合: 标量取首个非空, 列表整份取首个非空来源.
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
        let TaskPayload::Scrape { number } = context.payload else {
            return Err(TaskError::Failed("载荷类型与任务类型不匹配".to_owned()));
        };
        let number = MediaNumber::parse(&number)
            .map_err(|error| TaskError::Failed(format!("番号无法识别: {error}")))?;

        context.logger.info(format!("开始刮削 {number}")).await;

        // 只问当前类型链上的来源: 与类型不匹配的站点白跑一次请求, 还会拖慢整条链.
        let candidates = self.sources.chain_for(number.content_type());
        if candidates.is_empty() {
            let reason = format!("{} 没有可用的来源", number.content_type().label());
            context.logger.error(&reason).await;
            return Err(TaskError::Failed(reason));
        }
        context
            .logger
            .info(format!(
                "{} 的来源链: {}",
                number.content_type().label(),
                candidates
                    .iter()
                    .map(|source| source.descriptor().id)
                    .collect::<Vec<_>>()
                    .join(" → ")
            ))
            .await;

        // 并行请求: 单源失败不影响其它来源, 总耗时接近最慢来源.
        // 获取期间取消要立即生效: 等全部来源返回再检查, 取消会被拖到最后一个超时.
        let fetched = tokio::select! {
            results = futures::future::join_all(candidates.iter().map(|source| async {
                let id = source.descriptor().id;
                // 来源标识附加到 span: 该来源发出的每条上游日志都会携带它, 慢请求可按来源统计.
                let span = tracing::info_span!("source", source = id);
                let outcome = source.fetch(&number).instrument(span).await;
                (id, outcome)
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
            poster_urls: Some(&scraped.poster_urls),
            thumb_urls: Some(&scraped.thumb_urls),
            trailer_urls: Some(&scraped.trailer_urls),
            extrafanart_urls: Some(&scraped.extrafanart_urls),
            scores: Some(&scraped.scores),
            external_ids: Some(&scraped.external_ids),
            source_urls: Some(&scraped.source_urls),
            genres: Some(&scraped.genres),
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
        let snapshots: Vec<SourceSnapshotInput<'_>> = hits
            .iter()
            .map(|(id, scraped)| SourceSnapshotInput {
                source_id: id,
                payload: scraped.clone(),
            })
            .collect();
        self.metadata
            .replace_snapshots(row.id, &snapshots)
            .await
            .map_err(|error| TaskError::Failed(format!("写入来源快照失败: {error}")))?;
        // 图片下载拆成后继任务: 下载失败不该让刮削判失败, 拆开之后失败有独立记录与重试入口.
        Ok(TaskOutcome::with_followups(vec![TaskSpec::new(
            TaskPayload::Assets {
                number: row.number.clone(),
            },
        )]))
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
        TaskError, TaskHandler, TaskKind, TaskLogger, TaskOutcome, TaskPayload,
    };
    use async_trait::async_trait;
    use tokio_util::sync::CancellationToken;

    use super::ScrapeHandler;
    use amane_core::ContentType;
    use amane_core::config::ContentRoutes;
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
        let chain: Vec<&'static str> = sources
            .iter()
            .map(|source| source.descriptor().id)
            .collect();
        fixture_with_chain(sources, &chain).await
    }

    /// 每类都挂调用方给定的链: 这些用例关心聚合, 不关心类型差异.
    async fn fixture_with_chain(sources: Vec<Box<dyn Source>>, chain: &[&str]) -> Fixture {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let database = Database::open(&dir.path().join("amane.db"))
            .await
            .expect("应打开数据库");
        let metadata = MetadataRepository::new(database.clone());
        let registry = Registry::new(sources);
        let ids: Vec<String> = chain.iter().map(|id| (*id).to_owned()).collect();
        let routes = ContentType::ALL
            .into_iter()
            .fold(ContentRoutes::default(), |routes, kind| {
                routes.with_chain(kind, ids.clone())
            });
        registry.apply_routes(&routes);
        let handler = ScrapeHandler::new(Arc::new(registry), metadata.clone());

        Fixture {
            dir,
            handler,
            metadata,
        }
    }

    async fn run(
        handler: &ScrapeHandler,
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
    async fn chain_order_decides_which_title_wins() {
        let number = MediaNumber::parse("ABC-123").expect("应解析成功");

        for (chain, expected) in [
            (["first", "second"], "第一 ABC-123"),
            (["second", "first"], "第二 ABC-123"),
        ] {
            let fixture = fixture_with_chain(
                vec![
                    source("first", Some("第一"), false),
                    source("second", Some("第二"), false),
                ],
                &chain,
            )
            .await;

            run(
                &fixture.handler,
                fixture.dir.path(),
                TaskPayload::Scrape {
                    number: "ABC-123".to_owned(),
                },
            )
            .await
            .expect("应完成");

            let row = fixture
                .metadata
                .find(&number)
                .await
                .expect("应查询成功")
                .expect("应写入番号条目");
            assert_eq!(row.title.as_deref(), Some(expected), "链顺序 {chain:?}");
        }
    }

    #[tokio::test]
    async fn empty_chain_fails_with_reason() {
        let fixture = fixture_with_chain(vec![source("working", Some("命中"), false)], &[]).await;

        let result = run(
            &fixture.handler,
            fixture.dir.path(),
            TaskPayload::Scrape {
                number: "ABC-123".to_owned(),
            },
        )
        .await;

        match result {
            Err(TaskError::Failed(message)) => {
                assert!(message.contains("有码"), "应指出是哪一类: {message}");
                assert!(message.contains("没有可用的来源"), "应说明原因: {message}");
            }
            other => panic!("空链必须失败, 实际 {other:?}"),
        }
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
            TaskPayload::Scrape {
                number: "ABC-123".to_owned(),
            },
        )
        .await
        .expect("应完成");

        assert_eq!(outcome.followups.len(), 1, "刮削总会派生一个资源任务");
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
            TaskPayload::Scrape {
                number: "ABC-123".to_owned(),
            },
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
    async fn scrape_derives_assets_task() {
        let fixture = fixture(vec![source("working", Some("命中"), false)]).await;

        let outcome = run(
            &fixture.handler,
            fixture.dir.path(),
            TaskPayload::Scrape {
                number: "ABC-123".to_owned(),
            },
        )
        .await
        .expect("应完成");

        assert_eq!(outcome.followups.len(), 1, "刮削应派生一个资源任务");
        assert_eq!(outcome.followups[0].kind(), TaskKind::Assets);
        assert_eq!(
            outcome.followups[0].payload,
            TaskPayload::Assets {
                number: "ABC-123".to_owned()
            },
            "后继任务要带番号"
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
            TaskPayload::Scrape {
                number: "ABC-123".to_owned(),
            },
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
            TaskPayload::Scrape {
                number: "ABC-123".to_owned(),
            },
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
            TaskPayload::Scrape {
                number: "ABC-123".to_owned(),
            },
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
            TaskPayload::Scrape {
                number: "ABC-123".to_owned(),
            },
        )
        .await;

        match result {
            Err(TaskError::Failed(message)) => assert!(message.contains("没有来源返回结果")),
            _ => panic!("所有来源失败时必须失败并说明原因"),
        }
    }

    #[tokio::test]
    async fn rejects_unrecognized_number() {
        let fixture = fixture(vec![source("working", Some("命中"), false)]).await;

        let result = run(
            &fixture.handler,
            fixture.dir.path(),
            TaskPayload::Scrape {
                number: "???".to_owned(),
            },
        )
        .await;

        match result {
            Err(TaskError::Failed(message)) => assert!(message.contains("番号无法识别")),
            _ => panic!("非法番号必须失败并说明原因"),
        }
    }
}
