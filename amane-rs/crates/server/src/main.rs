use std::path::PathBuf;

use amane_core::config::Settings;
use amane_server::{AppState, Runtime, build_router};
use clap::Parser;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(name = "amane-server", about = "Amane 影片元数据服务")]
struct Args {
    /// 监听地址, 覆盖配置文件.
    #[arg(long)]
    bind: Option<String>,
    /// 数据目录, 覆盖配置文件.
    #[arg(long)]
    data_dir: Option<PathBuf>,
    /// 访问令牌; 指定后所有请求须携带 `Authorization: Bearer <token>`.
    #[arg(long)]
    token: Option<String>,
    /// 输出 OpenAPI 文档到标准输出后退出.
    #[arg(long)]
    print_openapi: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    if args.print_openapi {
        println!("{}", amane_server::openapi().to_pretty_json()?);
        return Ok(());
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let mut settings = Settings::default();
    if let Some(bind) = args.bind {
        settings.cold.bind = bind;
    }
    if let Some(data_dir) = args.data_dir {
        settings.cold.data_dir = data_dir;
    }
    if args.token.is_some() {
        settings.cold.api_token = args.token;
    }

    let runtime = Runtime::boot(settings).await?;
    let bind = runtime.settings().cold.bind.clone();
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    let address = listener.local_addr()?;
    tracing::info!(%address, "amane-server 已启动");

    let state = AppState::new(runtime);
    let worker = state.runtime.tasks.start();
    let scheduler_loop = state.runtime.scheduler.clone().spawn();

    axum::serve(listener, build_router(state.clone()))
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    // 出队循环经优雅停机结束; 调度循环在此结束. 登记与任务都已持久化, 重启后按基准时刻继续.
    state.runtime.tasks.shutdown();
    let _ = worker.await;
    scheduler_loop.abort();

    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("收到中断信号, 正在停止");
}
