//! 请求日志: 方法、路径、状态码与耗时.

use std::time::Instant;

use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;

/// 记录每个请求的完成情况.
///
/// 只记录路径, 不记录查询串: 桌面外壳用 `?token=` 传令牌, 记录查询串等于把令牌写进日志.
/// 级别固定为 INFO: 默认的 `EnvFilter` 是 `info`, 降到 DEBUG 的耗时日志在默认部署里看不到.
pub async fn log_requests(request: Request, next: Next) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let started = Instant::now();

    let response = next.run(request).await;

    tracing::info!(
        %method,
        %path,
        status = response.status().as_u16(),
        elapsed_ms = started.elapsed().as_millis() as u64,
        "请求完成"
    );

    response
}
