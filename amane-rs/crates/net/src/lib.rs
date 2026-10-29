//! 出站 HTTP: 浏览器指纹、按 host 限速、被拒后轮换指纹.

pub mod client;
pub mod limiter;

pub use client::{Fingerprint, NetError, RequestOptions, WebClient};
pub use limiter::{Rate, RateLimiters};
