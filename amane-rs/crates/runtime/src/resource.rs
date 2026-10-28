//! 资源物化: 下载 → 写入磁盘 → 登记.

use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use amane_core::{Resource, ResourceError, ResourceRepository};
use amane_net::WebClient;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error(transparent)]
    Repository(#[from] ResourceError),
    #[error("下载 {url} 失败: {message}")]
    Download { url: String, message: String },
    #[error("读写资源文件失败: {0}")]
    Io(#[from] io::Error),
}

/// 把远端资源取到本地并按地址去重.
pub struct ResourceStore {
    root: PathBuf,
    repository: ResourceRepository,
    client: Arc<WebClient>,
}

impl ResourceStore {
    pub fn new(root: PathBuf, repository: ResourceRepository, client: Arc<WebClient>) -> Self {
        Self {
            root,
            repository,
            client,
        }
    }

    pub fn repository(&self) -> &ResourceRepository {
        &self.repository
    }

    /// 取用一条资源; 已有写入磁盘文件时直接复用, 不重复下载.
    pub async fn materialize(&self, url: &str) -> Result<Resource, StoreError> {
        if let Some(existing) = self.repository.find_by_url(url).await?
            && !existing.file_name.is_empty()
        {
            return Ok(existing);
        }

        let (bytes, declared) =
            self.client
                .get_bytes(url)
                .await
                .map_err(|error| StoreError::Download {
                    url: url.to_owned(),
                    message: error.to_string(),
                })?;

        let (content_type, extension) = sniff(&bytes);
        if let Some(declared) = declared
            && !declared.starts_with(content_type)
        {
            tracing::warn!(
                %url,
                %declared,
                actual = content_type,
                "资源类型与上游声明不一致, 以实际内容为准"
            );
        }

        let size = i64::try_from(bytes.len()).unwrap_or(i64::MAX);
        let resource = self.repository.reserve(url, content_type, size).await?;
        let file_name = format!("{}{extension}", resource.id);

        tokio::fs::create_dir_all(&self.root).await?;
        if let Err(error) = tokio::fs::write(self.root.join(&file_name), &bytes).await {
            // 写入磁盘失败时撤销登记, 避免库里留下没有文件的资源.
            if let Err(cleanup) = self.repository.delete(resource.id).await {
                tracing::warn!(resource = resource.id, error = %cleanup, "撤销资源登记失败");
            }
            return Err(StoreError::Io(error));
        }

        self.repository
            .set_file_name(resource.id, &file_name)
            .await?;

        Ok(Resource {
            file_name,
            ..resource
        })
    }

    /// 读取已写入磁盘资源的内容.
    pub async fn read(&self, resource: &Resource) -> io::Result<Vec<u8>> {
        if resource.file_name.contains('/') || resource.file_name.contains("..") {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "资源文件名不合法",
            ));
        }

        tokio::fs::read(self.root.join(&resource.file_name)).await
    }
}

/// 按文件头判定类型与扩展名; 上游声明的类型只作参考.
fn sniff(bytes: &[u8]) -> (&'static str, &'static str) {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        ("image/jpeg", ".jpg")
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        ("image/png", ".png")
    } else if bytes.starts_with(b"GIF8") {
        ("image/gif", ".gif")
    } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        ("image/webp", ".webp")
    } else {
        ("application/octet-stream", ".bin")
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use amane_core::{Database, ResourceRepository};
    use amane_net::{Rate, RateLimiters, WebClient};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::{ResourceStore, StoreError, sniff};

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n0000000000";

    /// 只回固定内容的本地服务; 命中次数写入 `hits`.
    async fn spawn_stub(status: &'static str, body: &'static [u8]) -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("应绑定端口");
        let address = listener.local_addr().expect("应取得地址");
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&hits);

        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let mut buffer = [0_u8; 1024];
                let _ = socket.read(&mut buffer).await;
                counter.fetch_add(1, Ordering::SeqCst);
                let header = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(header.as_bytes()).await;
                let _ = socket.write_all(body).await;
                let _ = socket.shutdown().await;
            }
        });

        (format!("http://{address}/poster.jpg"), hits)
    }

    async fn store() -> (tempfile::TempDir, ResourceStore) {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let database = Database::open(&dir.path().join("amane.db"))
            .await
            .expect("应打开数据库");
        let client = Arc::new(
            WebClient::new(
                Duration::from_secs(5),
                RateLimiters::new(Rate::per_second(5, 20.0)),
            )
            .expect("客户端应构造成功"),
        );

        let store = ResourceStore::new(
            dir.path().join("resources"),
            ResourceRepository::new(database),
            client,
        );
        (dir, store)
    }

    #[tokio::test]
    async fn downloads_once_and_reuses_file() {
        let (url, hits) = spawn_stub("200 OK", PNG).await;
        let (_dir, store) = store().await;

        let first = store.materialize(&url).await.expect("应下载成功");
        let second = store.materialize(&url).await.expect("应复用成功");

        assert_eq!(first.id, second.id);
        assert_eq!(hits.load(Ordering::SeqCst), 1, "同一地址只应下载一次");
        assert_eq!(first.content_type, "image/png", "类型以实际内容为准");
        assert_eq!(
            store.read(&first).await.expect("应读取成功"),
            PNG,
            "磁盘内容应与响应一致"
        );
    }

    #[tokio::test]
    async fn download_failure_leaves_no_resource() {
        let (url, _hits) = spawn_stub("500 Internal Server Error", b"boom").await;
        let (_dir, store) = store().await;

        let result = store.materialize(&url).await;

        match result {
            Err(StoreError::Download { .. }) => {}
            other => panic!("上游失败时必须报下载失败, 实际 {other:?}"),
        }
        assert!(
            store
                .repository()
                .find_by_url(&url)
                .await
                .expect("应查询成功")
                .is_none(),
            "失败时不应留下资源记录"
        );
    }

    #[tokio::test]
    async fn rejects_illegal_file_name() {
        let (_dir, store) = store().await;
        for name in ["../escape", "nested/1.jpg", ""] {
            let resource = amane_core::Resource {
                id: 1,
                url: "https://example.invalid/x.jpg".to_owned(),
                file_name: name.to_owned(),
                content_type: "image/jpeg".to_owned(),
                size_bytes: 0,
                created_at: chrono::Utc::now(),
            };

            let result = store.read(&resource).await;

            assert!(
                result.is_err() || name.is_empty(),
                "文件名 {name:?} 不应被接受"
            );
        }
    }

    #[test]
    fn sniffs_by_magic_bytes() {
        let cases: &[(&[u8], &str)] = &[
            (&[0xFF, 0xD8, 0xFF, 0x00], "image/jpeg"),
            (b"\x89PNG\r\n\x1a\n", "image/png"),
            (b"GIF89a", "image/gif"),
            (b"RIFF0000WEBP", "image/webp"),
            (b"", "application/octet-stream"),
            (b"\x89PNG", "application/octet-stream"),
        ];

        for (bytes, expected) in cases {
            assert_eq!(sniff(bytes).0, *expected, "输入 {bytes:?}");
        }
    }
}
