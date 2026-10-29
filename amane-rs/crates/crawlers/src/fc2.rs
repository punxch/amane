//! FC2 官方电子市场. 地址由数字商品 ID 直接拼出, 不需要检索.
//!
//! 与 fc2ppvdb 的分工: 这条是官方站, 有官方简介与样品图; fc2ppvdb 是第三方库, 有演员与题材.

use std::sync::Arc;

use amane_core::MediaNumber;
use amane_net::{NetError, RequestOptions, WebClient};
use async_trait::async_trait;
use scraper::{Html, Selector};

use crate::dom::{absolute, attrs, find_date, texts};
use crate::{Capability, RawBody, ScrapedMetadata, Source, SourceDescriptor, SourceError};

/// 请求实际打到的主机名; 限速按它分桶.
pub const HOST: &str = "adult.contents.fc2.com";

const BASE_URL: &str = "https://adult.contents.fc2.com";

const ID: &str = "fc2";

/// 商品页独有的块. 商品不在售时站点回一个没有该块的「找不到商品」页 (HTTP 200).
const ARTICLE_BLOCK: &str = r#"div[data-section="userInfo"]"#;

pub struct Fc2Source {
    client: Arc<WebClient>,
}

impl Fc2Source {
    pub fn new(client: Arc<WebClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Source for Fc2Source {
    fn descriptor(&self) -> &SourceDescriptor {
        &DESCRIPTOR
    }

    async fn fetch(&self, number: &MediaNumber) -> Result<ScrapedMetadata, SourceError> {
        let Some(article_id) = article_id(number) else {
            return Err(SourceError::NotFound {
                id: ID,
                number: number.as_str().to_owned(),
            });
        };

        // 地址由商品 ID 拼出, 因此一次商品页请求同时充当「在售确认」与「取数」.
        let url = format!("{BASE_URL}/article/{article_id}/");
        let html = self
            .client
            .get_text_with(&url, &RequestOptions::new())
            .await
            .map_err(transport_error)?;

        parse_detail(&html, &article_id)
    }
}

static DESCRIPTOR: SourceDescriptor = SourceDescriptor {
    id: ID,
    label: "FC2",
    base_url: BASE_URL,
    host: HOST,
    rate: amane_net::Rate::each_second(1, 1),
    capabilities: &[Capability::Metadata],
};

fn transport_error(error: NetError) -> SourceError {
    SourceError::Transport {
        id: ID,
        message: error.to_string(),
    }
}

fn parse_failure(message: &str, html: &str) -> SourceError {
    SourceError::parse(
        ID,
        message.to_owned(),
        RawBody {
            content: html.to_owned(),
            extension: "html",
        },
    )
}

/// 番号里的数字商品 ID. 官方站写 `FC2-PPV-<id>`, 索引站多写 `FC2-<id>`; 两种都认.
fn article_id(number: &MediaNumber) -> Option<String> {
    let raw = number.as_str();
    let rest = raw.strip_prefix("FC2-")?;
    let digits = rest.strip_prefix("PPV-").unwrap_or(rest);

    (!digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| digits.to_owned())
}

fn parse_detail(html: &str, article_id: &str) -> Result<ScrapedMetadata, SourceError> {
    let document = Html::parse_document(html);

    // 不在售时页面是「找不到商品」, 与未收录同义; 缺别的块才算解析失败.
    let block = Selector::parse(ARTICLE_BLOCK).expect("固定选择器");
    if document.select(&block).next().is_none() {
        if html.contains("見つかりません") {
            return Err(SourceError::NotFound {
                id: ID,
                number: format!("FC2-PPV-{article_id}"),
            });
        }
        return Err(parse_failure("商品页没有用户信息块", html));
    }

    // 标题取主图 alt: 标题行里除标题外还有促销角标与站点插的隐藏水印, 只有 alt 是干净的.
    let Some(title) = attrs(&document, "div.items_article_MainitemThumb img", "alt")
        .into_iter()
        .next()
    else {
        return Err(parse_failure("商品页没有标题", html));
    };

    let samples = attrs(&document, "ul.items_article_SampleImagesArea li a", "href")
        .into_iter()
        .map(|href| absolute(BASE_URL, &href))
        .collect::<Vec<_>>();
    let poster = attrs(&document, "div.items_article_MainitemThumb span img", "src")
        .into_iter()
        .next()
        .map(|src| absolute(BASE_URL, &src));

    let mut scraped = ScrapedMetadata {
        title: Some(title),
        plot: attrs(&document, r#"meta[name="description"]"#, "content")
            .into_iter()
            .next(),
        release_date: texts(&document, "div.items_article_softDevice p")
            .iter()
            .find(|line| line.contains("販売日"))
            .and_then(|line| find_date(line)),
        maker: texts(&document, "div.items_article_headerInfo ul li a")
            .last()
            .cloned(),
        genres: texts(&document, "a.tag.tagTag"),
        poster_urls: poster.into_iter().collect(),
        // 样品图首张即横版封面, 其余进剧照.
        thumb_urls: samples.first().cloned().into_iter().collect(),
        ..ScrapedMetadata::default()
    };
    if !samples.is_empty() {
        scraped.extrafanart_urls.insert(ID.to_owned(), samples);
    }

    Ok(scraped)
}

#[cfg(test)]
mod tests {
    use amane_core::MediaNumber;

    use super::{article_id, parse_detail};

    const ARTICLE: &str = r#"
        <div class="items_article_header">
          <div class="items_article_MainitemThumb">
            <span><img src="//contents-thumbnail.fc2.com/poster.jpg" /></span>
            <img alt="商品标题" src="//contents-thumbnail.fc2.com/main.jpg" />
          </div>
          <div class="items_article_headerInfo">
            <ul><li><a href="/seller/1">卖家甲</a></li><li><a href="/seller/2">卖家乙</a></li></ul>
          </div>
          <div data-section="userInfo">卖家信息</div>
          <div class="items_article_softDevice">
            <p>販売日：2021/03/04</p>
            <p>再生時間：45分</p>
          </div>
          <ul class="items_article_SampleImagesArea">
            <li><a href="//contents-thumbnail.fc2.com/s1.jpg"></a></li>
            <li><a href="//contents-thumbnail.fc2.com/s2.jpg"></a></li>
          </ul>
          <a class="tag tagTag" href="/tag/1">素人</a>
          <a class="tag tagTag" href="/tag/2">企画</a>
          <meta name="description" content="官方简介" />
        </div>
    "#;

    /// 已下架: 站点回 200, 但页面是「找不到商品」.
    const WITHDRAWN: &str = r#"
        <html><body><div class="items_article_MainitemThumb"></div>
        <p>お探しの商品が見つかりませんでした</p></body></html>
    "#;

    /// 改版后的页面: 用户信息块还在, 标题没了.
    const CHANGED: &str = r#"<div data-section="userInfo">卖家信息</div>"#;

    fn number(raw: &str) -> MediaNumber {
        MediaNumber::parse(raw).expect("应解析成功")
    }

    #[test]
    fn reads_article_id_from_both_writings() {
        assert_eq!(
            article_id(&number("FC2-1234567")).as_deref(),
            Some("1234567")
        );
        assert_eq!(
            article_id(&number("FC2-PPV-1234567")).as_deref(),
            Some("1234567")
        );
        assert_eq!(article_id(&number("SSIS-001")), None, "非 FC2 番号不拼地址");
    }

    #[test]
    fn parses_article_fields() {
        let scraped = parse_detail(ARTICLE, "1234567").expect("应解析成功");

        assert_eq!(scraped.title.as_deref(), Some("商品标题"));
        assert_eq!(
            scraped.release_date.map(|date| date.to_string()),
            Some("2021-03-04".to_owned())
        );
        assert_eq!(scraped.maker.as_deref(), Some("卖家乙"), "卖家取最后一项");
        assert_eq!(scraped.genres, vec!["素人".to_owned(), "企画".to_owned()]);
        assert_eq!(scraped.plot.as_deref(), Some("官方简介"));
    }

    #[test]
    fn samples_become_cover_and_gallery() {
        let scraped = parse_detail(ARTICLE, "1234567").expect("应解析成功");

        assert_eq!(
            scraped.thumb_urls.first().map(String::as_str),
            Some("https://contents-thumbnail.fc2.com/s1.jpg"),
            "样品图首张即横版封面"
        );
        assert_eq!(
            scraped.poster_urls.first().map(String::as_str),
            Some("https://contents-thumbnail.fc2.com/poster.jpg")
        );
        assert_eq!(scraped.extrafanart_urls.get("fc2").map(Vec::len), Some(2));
    }

    #[test]
    fn withdrawn_article_is_not_found() {
        let error = parse_detail(WITHDRAWN, "1234567").expect_err("下架应判未收录");

        match error {
            super::SourceError::NotFound { number, .. } => {
                assert_eq!(number, "FC2-PPV-1234567", "报错用官方站的番号写法");
            }
            other => panic!("应判未收录, 实际 {other:?}"),
        }
    }

    #[test]
    fn changed_layout_keeps_raw() {
        let error = parse_detail(CHANGED, "1234567").expect_err("没有标题应判解析失败");

        match &error {
            super::SourceError::Parse { message, raw, .. } => {
                assert!(message.contains("标题"), "说明应指出缺什么: {message}");
                assert_eq!(raw.content, CHANGED);
                assert_eq!(raw.extension, "html");
            }
            other => panic!("应判解析失败, 实际 {other:?}"),
        }
    }
}
