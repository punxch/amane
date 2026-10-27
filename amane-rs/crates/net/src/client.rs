use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::RwLock;
use url::Url;
use wreq::Client;
use wreq_util::{Emulation, Platform, Profile};

use crate::limiter::RateLimiters;

/// 上游拒绝请求时返回的状态.
const REJECTED: &[u16] = &[403, 429, 503];

/// 浏览器档位. 被拒时按顺序轮换到下一个.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fingerprint {
    Chrome,
    Firefox,
    Safari,
}

/// 档位与平台必须配对: 浏览器与操作系统组合错位本身就是识别特征.
///
/// 这张表是新增档位的唯一位置; 轮换顺序即表内顺序.
const PROFILES: [(Fingerprint, Profile, Platform); 3] = [
    (Fingerprint::Chrome, Profile::Chrome149, Platform::MacOS),
    (Fingerprint::Firefox, Profile::Firefox151, Platform::Windows),
    (Fingerprint::Safari, Profile::Safari26_4, Platform::MacOS),
];

impl Fingerprint {
    fn emulation(self) -> Emulation {
        let profile = PROFILES
            .iter()
            .find(|entry| entry.0 == self)
            .map_or(Profile::Chrome149, |entry| entry.1);
        let platform = PROFILES
            .iter()
            .find(|entry| entry.0 == self)
            .map_or(Platform::MacOS, |entry| entry.2);

        Emulation::builder()
            .profile(profile)
            .platform(platform)
            .build()
    }

    fn next(self) -> Self {
        let index = PROFILES
            .iter()
            .position(|entry| entry.0 == self)
            .unwrap_or_default();
        PROFILES
            .get((index + 1) % PROFILES.len())
            .map_or(self, |entry| entry.0)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum NetError {
    #[error("构造 HTTP 客户端失败: {0}")]
    Build(String),
    #[error("URL 无法解析: {0}")]
    Url(String),
    #[error("请求 {url} 失败: {message}")]
    Request { url: String, message: String },
    #[error("上游返回 {status}, 轮换指纹后仍未通过")]
    Rejected { status: u16 },
}

/// 出站客户端: 每次请求先取该 host 的令牌, 被拒时轮换指纹后重试.
pub struct WebClient {
    /// 指纹变更需要重建客户端, 因此保存可替换的引用而不是固定实例.
    inner: RwLock<Arc<Client>>,
    profile: Mutex<Fingerprint>,
    limiters: RateLimiters,
    timeout: Duration,
    max_rotations: u32,
}

impl WebClient {
    pub fn new(timeout: Duration, limiters: RateLimiters) -> Result<Self, NetError> {
        let profile = Fingerprint::Chrome;
        let client = build(timeout, profile)?;

        Ok(Self {
            inner: RwLock::new(Arc::new(client)),
            profile: Mutex::new(profile),
            limiters,
            timeout,
            max_rotations: u32::try_from(PROFILES.len().saturating_sub(1)).unwrap_or(0),
        })
    }

    /// 当前使用的档位, 供日志与排查使用.
    pub fn fingerprint(&self) -> Fingerprint {
        *self
            .profile
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    pub async fn get_text(&self, url: &str) -> Result<String, NetError> {
        let response = self.get(url).await?;
        response
            .text()
            .await
            .map_err(|error| request_error(url, &error))
    }

    pub async fn get(&self, url: &str) -> Result<wreq::Response, NetError> {
        let host = host_of(url)?;
        let mut attempts = 0;

        loop {
            self.limiters.acquire(&host).await;
            let client = {
                let guard = self.inner.read().await;
                Arc::clone(&guard)
            };
            let response = client
                .get(url)
                .send()
                .await
                .map_err(|error| NetError::Request {
                    url: url.to_owned(),
                    message: error.to_string(),
                })?;

            let status = response.status().as_u16();
            if !REJECTED.contains(&status) {
                return Ok(response);
            }
            if attempts >= self.max_rotations {
                return Err(NetError::Rejected { status });
            }

            attempts += 1;
            tracing::warn!(%url, status, "上游拒绝请求, 轮换浏览器指纹");
            self.rotate().await?;
        }
    }

    /// 切换到下一个档位并重建客户端.
    async fn rotate(&self) -> Result<(), NetError> {
        let next = {
            let mut profile = self
                .profile
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            *profile = profile.next();
            *profile
        };

        let client = build(self.timeout, next)?;
        *self.inner.write().await = Arc::new(client);
        tracing::info!(profile = ?next, "已切换浏览器指纹");

        Ok(())
    }
}

fn build(timeout: Duration, profile: Fingerprint) -> Result<Client, NetError> {
    Client::builder()
        .emulation(profile.emulation())
        .timeout(timeout)
        .build()
        .map_err(|error| NetError::Build(error.to_string()))
}

fn request_error(url: &str, error: &wreq::Error) -> NetError {
    NetError::Request {
        url: url.to_owned(),
        message: error.to_string(),
    }
}

fn host_of(url: &str) -> Result<String, NetError> {
    let parsed = Url::parse(url).map_err(|error| NetError::Url(error.to_string()))?;
    parsed
        .host_str()
        .map(str::to_owned)
        .ok_or_else(|| NetError::Url(format!("{url} 没有主机名")))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{Fingerprint, NetError, WebClient, host_of};
    use crate::limiter::{Rate, RateLimiters};

    fn client_with(burst: u32, per_second: f64) -> WebClient {
        WebClient::new(
            Duration::from_secs(5),
            RateLimiters::new(Rate::per_second(burst, per_second)),
        )
        .expect("客户端应构造成功")
    }

    fn client() -> WebClient {
        client_with(2, 2.0)
    }

    #[test]
    fn rotates_through_every_profile_and_returns_to_first() {
        assert_eq!(Fingerprint::Chrome.next(), Fingerprint::Firefox);
        assert_eq!(Fingerprint::Firefox.next(), Fingerprint::Safari);
        assert_eq!(Fingerprint::Safari.next(), Fingerprint::Chrome);
    }

    #[test]
    fn reads_host_from_url() {
        let cases: &[(&str, Option<&str>)] = &[
            ("https://javdb.com/v/abc", Some("javdb.com")),
            ("http://127.0.0.1:8080/x", Some("127.0.0.1")),
            ("not a url", None),
            ("", None),
        ];

        for (input, expected) in cases {
            let actual = host_of(input).ok();
            assert_eq!(actual.as_deref(), *expected, "输入 {input}");
        }
    }

    #[tokio::test]
    async fn starts_on_chrome_then_rotates() {
        let client = client();
        assert_eq!(client.fingerprint(), Fingerprint::Chrome);

        client.rotate().await.expect("应轮换成功");

        assert_eq!(client.fingerprint(), Fingerprint::Firefox);
    }

    /// 启动一个只回固定文本的本地服务; 前 `rejects` 次回 403, 之后回 200.
    async fn spawn_stub(rejects: usize) -> (String, tokio::task::JoinHandle<usize>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.expect("应绑定端口");
        let address = listener.local_addr().expect("应取得地址");
        let handle = tokio::spawn(async move {
            let mut served = 0;
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let mut buffer = [0_u8; 1024];
                let _ = socket.read(&mut buffer).await;
                let status = if served < rejects {
                    "403 Forbidden"
                } else {
                    "200 OK"
                };
                served += 1;
                let response = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: text/plain\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok"
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            }
            served
        });

        (format!("http://{address}/"), handle)
    }

    #[tokio::test]
    async fn rotates_on_rejection_and_retries() {
        let (url, server) = spawn_stub(1).await;
        let client = client_with(5, 20.0);

        let body = client.get_text(&url).await.expect("轮换后应取到内容");

        assert_eq!(body, "ok");
        assert_eq!(
            client.fingerprint(),
            Fingerprint::Firefox,
            "被拒后应切换档位"
        );
        server.abort();
    }

    #[tokio::test]
    async fn reports_rejection_when_every_profile_fails() {
        let (url, server) = spawn_stub(10).await;
        let client = client_with(5, 20.0);

        let result = client.get_text(&url).await;

        match result {
            Err(NetError::Rejected { status }) => assert_eq!(status, 403),
            other => panic!("全档位被拒时必须报 Rejected, 实际 {other:?}"),
        }
        server.abort();
    }
}
