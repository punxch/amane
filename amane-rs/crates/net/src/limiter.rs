//! 按 host 的令牌桶.
//!
//! 目标不是精确的流量整形, 而是把同一站点的并发压到不会触发封禁的程度:
//! 容量即突发上限, 之后按固定间隔匀速补充.
//!
//! 速率表可在运行期整体替换: 来源在编译期声明自己的速率, 配置再按 host 覆盖,
//! 两者都变化于进程启动之后, 因此不能只留一个构造期入参.

use std::collections::HashMap;
use std::sync::{Mutex, RwLock};
use std::time::{Duration, Instant};

/// 单个 host 的速率.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rate {
    /// 突发上限, 也是桶容量.
    pub burst: u32,
    /// 补充一个令牌的间隔.
    pub interval: Duration,
}

impl Rate {
    /// 每秒 `per_second` 个请求, 允许 `burst` 个瞬时并发.
    pub fn per_second(burst: u32, per_second: f64) -> Self {
        let interval = if per_second > 0.0 {
            Duration::from_secs_f64(1.0 / per_second)
        } else {
            Duration::from_secs(1)
        };

        Self {
            burst: burst.max(1),
            interval,
        }
    }

    /// 每秒整数次的常量构造, 供来源在静态声明里写速率.
    pub const fn each_second(burst: u32, per_second: u32) -> Self {
        let per_second = if per_second == 0 { 1 } else { per_second };
        Self {
            burst: if burst == 0 { 1 } else { burst },
            interval: Duration::from_millis(1000 / per_second as u64),
        }
    }

    /// 两份速率里更保守的一份: 间隔取大, 突发取小.
    ///
    /// 同一 host 上有多个来源时用得上 —— 谁更慢就按谁走, 否则快的那份会让站点看到
    /// 一个它没答应的频率.
    pub fn slower(self, other: Self) -> Self {
        Self {
            burst: self.burst.min(other.burst),
            interval: self.interval.max(other.interval),
        }
    }
}

/// 一次取令牌的结果.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Grant {
    Acquired,
    Retry(Duration),
}

#[derive(Debug)]
struct Bucket {
    tokens: f64,
    last: Instant,
}

/// 按 host 分桶的限速器.
#[derive(Debug)]
pub struct RateLimiters {
    /// 未单独声明的 host 使用的速率; 图片 CDN 一类不经过来源的地址落在它上面.
    default: RwLock<Rate>,
    overrides: RwLock<HashMap<String, Rate>>,
    buckets: Mutex<HashMap<String, Bucket>>,
}

impl RateLimiters {
    pub fn new(default: Rate) -> Self {
        Self::with_table(default, HashMap::new())
    }

    /// 以一份完整的 host 速率表构造.
    pub fn with_table(default: Rate, table: HashMap<String, Rate>) -> Self {
        Self {
            default: RwLock::new(default),
            overrides: RwLock::new(table),
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// 为特定 host 覆盖默认速率.
    pub fn with_override(mut self, host: impl Into<String>, rate: Rate) -> Self {
        let table = self
            .overrides
            .get_mut()
            .unwrap_or_else(|error| error.into_inner());
        table.insert(host.into(), rate);
        self
    }

    /// 整体替换速率表.
    ///
    /// 已建立的桶保留令牌数, 下一次取令牌即按新速率补充, 因此换表不需要重建客户端.
    pub fn replace(&self, default: Rate, table: HashMap<String, Rate>) {
        *self
            .default
            .write()
            .unwrap_or_else(|error| error.into_inner()) = default;
        *self
            .overrides
            .write()
            .unwrap_or_else(|error| error.into_inner()) = table;
    }

    fn rate_for(&self, host: &str) -> Rate {
        self.overrides
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .get(host)
            .copied()
            .unwrap_or_else(|| {
                *self
                    .default
                    .read()
                    .unwrap_or_else(|error| error.into_inner())
            })
    }

    /// 取得一个令牌, 不足时等待.
    pub async fn acquire(&self, host: &str) {
        loop {
            match self.take(host) {
                Grant::Acquired => return,
                Grant::Retry(delay) => tokio::time::sleep(delay).await,
            }
        }
    }

    /// 尝试获取一个令牌.
    fn take(&self, host: &str) -> Grant {
        let rate = self.rate_for(host);
        let now = Instant::now();

        // 锁定期间没有会 panic 的代码, 中毒时取出内部值继续使用.
        let mut buckets = self
            .buckets
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let bucket = buckets.entry(host.to_owned()).or_insert(Bucket {
            tokens: f64::from(rate.burst),
            last: now,
        });

        let elapsed = now.saturating_duration_since(bucket.last);
        let refilled = elapsed.as_secs_f64() / rate.interval.as_secs_f64();
        bucket.tokens = (bucket.tokens + refilled).min(f64::from(rate.burst));
        bucket.last = now;

        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            return Grant::Acquired;
        }

        Grant::Retry(rate.interval.mul_f64(1.0 - bucket.tokens))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::time::{Duration, Instant};

    use super::{Rate, RateLimiters};

    #[tokio::test]
    async fn burst_passes_immediately_then_throttles() {
        let limiters = RateLimiters::new(Rate::per_second(2, 20.0));
        let started = Instant::now();

        limiters.acquire("example.com").await;
        limiters.acquire("example.com").await;
        let burst = started.elapsed();

        limiters.acquire("example.com").await;
        let total = started.elapsed();

        assert!(
            burst < Duration::from_millis(30),
            "突发额度不应等待, 实际 {burst:?}"
        );
        assert!(
            total >= Duration::from_millis(40),
            "超出突发后必须等待, 实际 {total:?}"
        );
    }

    #[tokio::test]
    async fn hosts_are_independent() {
        let limiters = RateLimiters::new(Rate::per_second(1, 1.0));
        let started = Instant::now();

        limiters.acquire("a.example").await;
        limiters.acquire("b.example").await;

        assert!(
            started.elapsed() < Duration::from_millis(50),
            "不同 host 不应互相阻塞"
        );
    }

    #[test]
    fn rate_clamps_zero_inputs() {
        let zero_burst = Rate::per_second(0, 5.0);
        let zero_rate = Rate::per_second(1, 0.0);

        assert_eq!(zero_burst.burst, 1, "容量至少为 1");
        assert_eq!(
            zero_rate.interval,
            Duration::from_secs(1),
            "零速率退化为每秒一个"
        );
    }

    #[tokio::test]
    async fn zero_per_second_still_grants_eventually() {
        let limiters = RateLimiters::new(Rate::per_second(1, 0.0));

        limiters.acquire("slow.example").await;
        limiters.acquire("slow.example").await;
    }

    #[tokio::test]
    async fn overrides_apply_to_named_host() {
        let limiters = RateLimiters::new(Rate::per_second(5, 100.0))
            .with_override("slow.example", Rate::per_second(1, 2.0));
        let started = Instant::now();

        limiters.acquire("slow.example").await;
        limiters.acquire("slow.example").await;

        assert!(
            started.elapsed() >= Duration::from_millis(400),
            "覆盖速率应生效, 实际 {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn slower_takes_the_more_conservative_side() {
        let slow = Rate::each_second(1, 1);
        let fast = Rate::each_second(5, 20);

        let merged = slow.slower(fast);

        assert_eq!(merged.burst, 1, "突发取小");
        assert_eq!(merged.interval, Duration::from_secs(1), "间隔取大");
        assert_eq!(merged, fast.slower(slow), "与顺序无关");
    }

    #[tokio::test]
    async fn replaced_table_applies_to_later_requests() {
        let limiters = RateLimiters::new(Rate::per_second(5, 100.0));
        let mut table = HashMap::new();
        table.insert("slow.example".to_owned(), Rate::each_second(1, 2));
        limiters.replace(Rate::per_second(5, 100.0), table);

        let started = Instant::now();
        limiters.acquire("slow.example").await;
        limiters.acquire("slow.example").await;

        assert!(
            started.elapsed() >= Duration::from_millis(400),
            "换表后新速率应生效, 实际 {:?}",
            started.elapsed()
        );
    }
}
