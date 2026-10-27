//! 任务处理函数的实现.
//!
//! 跨 crate 的编排放在这里: `amane-core` 定义任务契约, `amane-crawlers` 只回答「番号 → 元数据」,
//! 把两者接起来并写入持久化的代码在本 crate.

pub mod scrape;

pub use scrape::ScrapeHandler;
