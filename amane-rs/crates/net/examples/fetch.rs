//! 指纹连通性检查: 获取一个 URL, 打印状态与正文前缀.
//!
//! 用法: cargo run -p amane-net --example fetch -- <url>

use std::time::{Duration, Instant};

use amane_net::{Rate, RateLimiters, WebClient};

#[tokio::main]
async fn main() {
    let Some(url) = std::env::args().nth(1) else {
        eprintln!("用法: fetch <url>");
        return;
    };

    let client = match WebClient::new(
        Duration::from_secs(20),
        RateLimiters::new(Rate::per_second(1, 1.0)),
    ) {
        Ok(client) => client,
        Err(error) => {
            eprintln!("构造客户端失败: {error}");
            return;
        }
    };

    let started = Instant::now();
    match client.get_text(&url).await {
        Ok(body) => {
            let preview: String = body.chars().take(120).collect();
            println!(
                "ok {} 字节 / {:?} / 指纹 {:?}\n{}",
                body.len(),
                started.elapsed(),
                client.fingerprint(),
                preview.replace('\n', " ")
            );
        }
        Err(error) => println!("err: {error}"),
    }
}
